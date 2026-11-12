//! Extension feed: the overlay as a WebSocket **server**.
//!
//! The bridge extension (extension/) runs inside music.apple.com, reads the player
//! internals there and connects to `ws://127.0.0.1:<port>/<token>`.
//!
//! Why a server on our side: a Chrome extension cannot write into our process, and
//! polling a local HTTP endpoint from a content script runs into CSP and CORS; a
//! WebSocket initiated from the extension's isolated world is clean.
//!
//! Everything is loopback-only and gated by a token in the path, because any local
//! process could otherwise feed us bogus playback data.

use super::{PlaybackSnapshot, PlaybackState, Provider, TrackInfo};
use serde::Deserialize;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tungstenite::Message;

#[derive(Debug, Clone, Deserialize)]
pub struct StatePayload {
    pub playing: bool,
    pub position: Option<f64>,
    pub duration: Option<f64>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    #[serde(rename = "songId")]
    pub song_id: Option<String>,
    #[serde(default)]
    pub storefront: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Message2 {
    State { payload: StatePayload },
    Lyrics { #[serde(rename = "songId")] song_id: Option<String>, ttml: String },
}

#[derive(Default)]
pub struct Feed {
    pub snapshot: Option<PlaybackSnapshot>,
    pub seen_at: Option<Instant>,
    /// (song id, TTML) as delivered by the page.
    pub lyrics: Option<(String, String)>,
    pub lyric_seq: u64,
    pub connected: bool,
}

pub type SharedFeed = Arc<Mutex<Feed>>;

pub fn new_feed() -> SharedFeed {
    Arc::new(Mutex::new(Feed::default()))
}

/// Bind once and serve clients forever. Reconnects from the page are normal (the
/// extension reconnects on navigation), so the loop accepts repeatedly.
pub fn start_server(port: u16, token: String, feed: SharedFeed) {
    std::thread::spawn(move || {
        let listener = match TcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => l,
            Err(err) => {
                eprintln!("[ext] cannot bind 127.0.0.1:{port}: {err}");
                return;
            }
        };
        println!("[ext] listening on ws://127.0.0.1:{port}/<token>");

        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let feed = feed.clone();
            let expected_path = format!("/{token}");
            std::thread::spawn(move || {
                let handshake = tungstenite::accept_hdr(
                    stream,
                    |req: &tungstenite::handshake::server::Request, resp: tungstenite::handshake::server::Response| {
                        if req.uri().path() != expected_path {
                            let mut error = tungstenite::handshake::server::ErrorResponse::new(Some(
                                "bad token".to_string(),
                            ));
                            *error.status_mut() = tungstenite::http::StatusCode::FORBIDDEN;
                            return Err(error);
                        }
                        Ok(resp)
                    },
                );
                let mut socket = match handshake {
                    Ok(ws) => ws,
                    Err(err) => {
                        eprintln!("[ext] handshake rejected: {err}");
                        return;
                    }
                };
                println!("[ext] extension connected");
                if let Ok(mut guard) = feed.lock() {
                    guard.connected = true;
                }

                loop {
                    match socket.read() {
                        Ok(Message::Text(text)) => apply(&feed, text.as_str()),
                        Ok(Message::Binary(bytes)) => {
                            apply(&feed, &String::from_utf8_lossy(&bytes));
                        }
                        Ok(Message::Close(_)) => break,
                        Ok(_) => continue,
                        Err(_) => break,
                    }
                }
                if let Ok(mut guard) = feed.lock() {
                    guard.connected = false;
                    guard.snapshot = None;
                    guard.seen_at = None;
                }
                println!("[ext] extension disconnected");
            });
        }
    });
}

fn apply(feed: &SharedFeed, text: &str) {
    let Ok(message) = serde_json::from_str::<Message2>(text) else {
        return;
    };
    let Ok(mut guard) = feed.lock() else { return };

    match message {
        Message2::State { payload } => {
            if payload.title.as_deref().unwrap_or("").is_empty() {
                return;
            }
            guard.snapshot = Some(PlaybackSnapshot {
                track: Some(TrackInfo {
                    title: payload.title.unwrap_or_default(),
                    artist: payload.artist.unwrap_or_default(),
                    album: payload.album.unwrap_or_default(),
                    app_id: "extension".into(),
                    song_id: payload.song_id,
                }),
                state: if payload.playing { PlaybackState::Playing } else { PlaybackState::Paused },
                position_secs: payload.position.unwrap_or(0.0),
                duration_secs: payload.duration.unwrap_or(0.0),
                // The page reports live values on every tick.
                report_age_ms: 0,
                source: "extension".into(),
            });
            guard.seen_at = Some(Instant::now());
        }
        Message2::Lyrics { song_id, ttml } => {
            if ttml.len() < 32 {
                return;
            }
            guard.lyrics = Some((song_id.unwrap_or_default(), ttml));
            guard.lyric_seq += 1;
        }
    }
}

pub struct ExtensionProvider {
    feed: SharedFeed,
    enabled: bool,
}

impl ExtensionProvider {
    pub fn new(feed: SharedFeed, enabled: bool) -> Self {
        Self { feed, enabled }
    }
}

impl Provider for ExtensionProvider {
    fn id(&self) -> &'static str {
        "extension"
    }

    fn precision(&self) -> u8 {
        2
    }

    fn poll(&self) -> Option<PlaybackSnapshot> {
        if !self.enabled {
            return None;
        }
        let guard = self.feed.lock().ok()?;
        let seen = guard.seen_at?;
        // The page sends every 200 ms; anything older than 2 s means it stopped.
        if seen.elapsed() > Duration::from_secs(2) {
            return None;
        }
        guard.snapshot.clone()
    }
}
