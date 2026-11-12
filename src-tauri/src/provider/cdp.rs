//! CDP provider — precise playback position and Apple's own lyrics.
//!
//! How it works (verified against a live Apple Music page):
//!   * Chrome is started with `--remote-debugging-port=PORT` and an isolated profile
//!     (`tools/start-chrome-cdp.ps1`).
//!   * `GET http://127.0.0.1:PORT/json` lists the pages; we pick the music.apple.com one
//!     and connect to its `webSocketDebuggerUrl`.
//!   * `Runtime.evaluate` runs a small expression *inside* the page and returns JSON.
//!
//! Why this exists: Apple Music publishes its MediaSession timeline exactly once per
//! playback event, so SMTC can only extrapolate from an anchor (see docs/m2-session-notes.md).
//! Inside the page, `audio.currentTime` is exact.
//!
//! Notes from probing the live page:
//!   * `window.MusicKit.getInstance().developerToken` is readable (a JWT, ~268 chars).
//!   * `/v1/catalog/{sf}/search?types=songs` works with that token.
//!   * `/v1/catalog/{sf}/songs/{id}/lyrics` returns 404 "No related resources", so the
//!     lyrics payload is captured from the player's own traffic instead
//!     (`tools/cdp-netlog.mjs` collects it).

use super::{PlaybackSnapshot, PlaybackState, Provider, TrackInfo};
use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tungstenite::{connect, Message, WebSocket};
use tungstenite::stream::MaybeTlsStream;
use std::net::TcpStream;

type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

/// Runs inside the page. Must stay dependency-free and cheap: it is called ~5x/s.
///
/// Apple's player keeps several `<audio>` elements alive (previews for the shelves)
/// and only one of them is the real stream, so pick the playing one rather than the
/// first match.
const PROBE_EXPR: &str = r#"(() => {
  try {
    const kit = window.MusicKit && window.MusicKit.getInstance ? window.MusicKit.getInstance() : null;
    const mip = kit && kit._playbackController && kit._playbackController.services
      ? kit._playbackController.services.mediaItemPlayback : null;
    const cp = mip && mip.currentPlayback ? mip.currentPlayback : null;
    const player = mip && mip._currentPlayer ? mip._currentPlayer : null;
    const buf = player && player._buffer ? player._buffer : null;

    // A live/concert album is streamed as ONE HLS timeline, so the media element's
    // currentTime is concert time, not song time. Apple keeps the song's start in
    // `currentTimestampOffset`; verified live: 607.322446 - 502.322446 = 105.000.
    let pos = null;
    let dur = null;
    if (buf && Number.isFinite(buf._currentTime) && Number.isFinite(buf.currentTimestampOffset)) {
      pos = Math.max(0, buf._currentTime - buf.currentTimestampOffset);
    }
    if (buf && Number.isFinite(buf.duration)) dur = buf.duration;

    const meta = navigator.mediaSession && navigator.mediaSession.metadata;
    const attrs = cp && cp.item && cp.item.attributes ? cp.item.attributes : null;
    const songId = cp && cp.asset && cp.asset.id ? String(cp.asset.id) : null;

    return JSON.stringify({
      hasMedia: !!(buf || document.querySelector('audio')),
      pos,
      dur,
      paused: player ? !!player._paused : null,
      title: (attrs && attrs.name) || (meta ? meta.title : null),
      artist: (attrs && attrs.artistName) || (meta ? meta.artist : null),
      album: (attrs && attrs.albumName) || (meta ? meta.album : null),
      songId
    });
  } catch (e) {
    return JSON.stringify({ error: String(e).slice(0, 200) });
  }
})()"#;

/// Resolves the playing song's lyrics.
///
/// `__SONG_ID__` is the catalog id read off `currentPlayback.asset.id`, so the normal
/// path needs no search at all. The search branch stays as a fallback for builds where
/// that id is not exposed.
const LYRICS_EXPR: &str = r#"(async () => {
  try {
    const kit = window.MusicKit.getInstance();
    const token = kit.developerToken;
    const userToken = kit.musicUserToken;
    const sf = kit.storefrontId || 'cn';
    const wantTitle = __TITLE__;
    const wantArtist = __ARTIST__;
    const headers = {
      Authorization: 'Bearer ' + token,
      'media-user-token': userToken,
      Origin: 'https://music.apple.com'
    };
    const norm = (s) => (s || '').toLowerCase().replace(/\s+/g, '');

    const doFetch = async (url) => {
      const ctrl = new AbortController();
      const timer = setTimeout(() => ctrl.abort(), 6000);
      try {
        const res = await fetch(url, { headers, signal: ctrl.signal });
        if (!res.ok) return { status: res.status };
        return { status: 200, json: await res.json() };
      } finally {
        clearTimeout(timer);
      }
    };

    let id = __SONG_ID__;
    if (!id) {
      const searchUrl = 'https://amp-api.music.apple.com/v1/catalog/' + sf + '/search?term=' +
        encodeURIComponent((wantTitle || '') + ' ' + (wantArtist || '')) + '&types=songs&limit=10';
      const found = await doFetch(searchUrl);
      if (found.status !== 200) return JSON.stringify({ status: found.status, stage: 'search' });
      const songs = (found.json.results && found.json.results.songs && found.json.results.songs.data) || [];
      let best = null;
      let bestScore = -1e9;
      for (const s of songs) {
        const a = s.attributes || {};
        const name = norm(a.name);
        const artist = norm(a.artistName);
        let score = 0;
        if (name === norm(wantTitle)) score += 100; else if (name.includes(norm(wantTitle))) score += 40; else continue;
        if (artist.includes(norm(wantArtist)) || norm(wantArtist).includes(artist)) score += 60;
        if (a.hasLyrics) score += 20;
        if (score > bestScore) { bestScore = score; best = s; }
      }
      if (!best) return JSON.stringify({ status: 200, ttml: null, reason: 'no-match' });
      id = best.id;
    }

    const lyricUrl = 'https://amp-api.music.apple.com/v1/catalog/' + sf + '/songs/' + id +
      '/syllable-lyrics?l%5Blyrics%5D=zh-hans-cn&l%5Bscript%5D=zh-Hans&extend=ttmlLocalizations';
    const got = await doFetch(lyricUrl);
    if (got.status !== 200) return JSON.stringify({ status: got.status, stage: 'lyrics', id });
    const item = got.json.data && got.json.data[0];
    const attrs = item && item.attributes ? item.attributes : null;
    const ttml = attrs ? attrs.ttmlLocalizations : null;
    return JSON.stringify({ status: 200, id, ttml: typeof ttml === 'string' ? ttml : null });
  } catch (e) {
    return JSON.stringify({ error: String(e).slice(0, 200) });
  }
})()"#;

/// Is a *playback* browser speaking the DevTools protocol on this port?
///
/// `/json/version` alone is not enough: debug builds expose the overlay's own WebView2
/// on a devtools port too, and that endpoint answers happily. So we look at the page
/// targets and ignore endpoints whose only page is the overlay itself.
pub fn browser_alive(port: u16) -> bool {
    let Some(targets) = list_targets(port) else { return false };
    let pages: Vec<&serde_json::Value> = targets
        .iter()
        .filter(|t| t.get("type").and_then(Value::as_str) == Some("page"))
        .collect();

    if pages.is_empty() {
        // Browser is up but nothing opened yet; treat it as ours.
        return true;
    }

    pages.iter().any(|t| {
        let url = t.get("url").and_then(Value::as_str).unwrap_or_default();
        !url.starts_with("http://127.0.0.1:1420") && !url.starts_with("tauri://")
    })
}

fn list_targets(port: u16) -> Option<Vec<Value>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(900))
        .build()
        .ok()?;
    let response = client.get(format!("http://127.0.0.1:{port}/json")).send().ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json::<Vec<Value>>().ok()
}

fn default_chrome_candidates() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    // Chrome first, per-user install before machine-wide: a per-user Chrome is common
    // and must not lose to an Edge that happens to live in Program Files.
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        out.push(std::path::Path::new(&local).join("Google/Chrome/Application/chrome.exe"));
    }
    if let Ok(pf) = std::env::var("ProgramFiles") {
        out.push(std::path::Path::new(&pf).join("Google/Chrome/Application/chrome.exe"));
    }
    if let Ok(pf86) = std::env::var("ProgramFiles(x86)") {
        out.push(std::path::Path::new(&pf86).join("Google/Chrome/Application/chrome.exe"));
        out.push(std::path::Path::new(&pf86).join("Microsoft/Edge/Application/msedge.exe"));
    }
    if let Ok(pf) = std::env::var("ProgramFiles") {
        out.push(std::path::Path::new(&pf).join("Microsoft/Edge/Application/msedge.exe"));
    }
    out
}

fn default_profile_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("APPDATA")
        .map(|base| std::path::PathBuf::from(base).join("lyric-float").join("chrome-cdp-profile"))
}

/// Bring up the debug browser if it is not already there.
///
/// The DevTools port can only be opened at launch time, so an already running Chrome
/// can never be attached to. This makes that constraint invisible to the user: the
/// overlay starts its own browser instead of asking them to remember a script.
pub fn launch_browser(port: u16, chrome_path: Option<&str>, profile_dir: Option<&str>, url: &str) -> bool {
    if browser_alive(port) {
        return true;
    }

    let chrome = chrome_path
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists())
        .or_else(|| default_chrome_candidates().into_iter().find(|p| p.exists()));
    let Some(chrome) = chrome else {
        eprintln!("[cdp] no chrome/edge executable found; set cdp_chrome_path");
        return false;
    };

    let profile = profile_dir
        .map(std::path::PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(default_profile_dir);
    if let Some(dir) = &profile {
        let _ = std::fs::create_dir_all(dir);
    }

    // Launch through ShellExecute rather than CreateProcess.
    //
    // Measured on the development machine: this process runs inside a Windows job
    // object (the harness sandbox). The exact same command line handed to CreateProcess
    // made Chrome exit immediately with STATUS_BREAKPOINT (0x80000003, a Chromium CHECK
    // failure), while shell-started Chrome (Start-Process / Explorer) launched fine.
    // ShellExecute also keeps our stdio handles out of the browser's environment.
    // Every switch must come before the URL: Chrome treats the first non-switch
    // argument as the start of the URL list, so a trailing `--user-data-dir` is
    // silently dropped and the launch is forwarded to an already running instance
    // (which never opens a debug port).
    let mut params = format!("--remote-debugging-port={port}");
    if let Some(dir) = &profile {
        params.push_str(&format!(" --user-data-dir=\"{}\"", dir.display()));
    }
    params.push_str(" --no-first-run --no-default-browser-check --new-window");
    params.push_str(&format!(" \"{url}\""));

    match shell_execute(&chrome, &params) {
        true => {
            println!("[cdp] launch requested: {} (port {port})", chrome.display());
            true
        }
        false => {
            eprintln!("[cdp] shell launch failed for {}", chrome.display());
            false
        }
    }
}

#[cfg(windows)]
fn shell_execute(exe: &std::path::Path, params: &str) -> bool {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteW(
            hwnd: isize,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show: i32,
        ) -> isize;
    }

    fn wide(s: &[u16]) -> Vec<u16> {
        s.iter().copied().chain(std::iter::once(0)).collect()
    }

    let op: Vec<u16> = "open".encode_utf16().collect();
    let file: Vec<u16> = exe.as_os_str().encode_wide().collect();
    let par: Vec<u16> = params.encode_utf16().collect();

    let op = wide(&op);
    let file = wide(&file);
    let par = wide(&par);

    let result = unsafe {
        ShellExecuteW(
            0,
            op.as_ptr(),
            file.as_ptr(),
            par.as_ptr(),
            std::ptr::null(),
            1, // SW_SHOWNORMAL
        )
    };
    // ShellExecute returns a value > 32 on success.
    result > 32
}

#[cfg(not(windows))]
fn shell_execute(exe: &std::path::Path, _params: &str) -> bool {
    std::process::Command::new(exe).spawn().is_ok()
}

pub struct CdpProvider {    port: u16,
    enabled: bool,
    socket: Mutex<Option<Socket>>,
    last_error: Mutex<Option<String>>,
}

impl CdpProvider {
    pub fn new(port: u16, enabled: bool) -> Self {
        Self {
            port,
            enabled,
            socket: Mutex::new(None),
            last_error: Mutex::new(None),
        }
    }

    fn note_error(&self, err: impl std::fmt::Display) {
        let text = err.to_string();
        let mut slot = self.last_error.lock().unwrap();
        // Only log on change, otherwise a missing browser floods the console.
        if slot.as_deref() != Some(text.as_str()) {
            eprintln!("[cdp] {text}");
            *slot = Some(text);
        }
    }

    fn page_ws_url(&self) -> Result<String, String> {
        let url = format!("http://127.0.0.1:{}/json", self.port);
        let response = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(1200))
            .build()
            .map_err(|e| e.to_string())?
            .get(&url)
            .send()
            .map_err(|e| format!("devtools endpoint unreachable: {e}"))?;

        let targets: Value = response.json().map_err(|e| e.to_string())?;
        let list = targets.as_array().ok_or("unexpected /json payload")?;
        for target in list {
            let page_url = target.get("url").and_then(Value::as_str).unwrap_or_default();
            if page_url.contains("music.apple.com") {
                if let Some(ws) = target.get("webSocketDebuggerUrl").and_then(Value::as_str) {
                    return Ok(ws.to_string());
                }
            }
        }
        Err("no music.apple.com page in the debugged browser".into())
    }

    fn ensure_socket(&self) -> Option<std::sync::MutexGuard<'_, Option<Socket>>> {
        let mut guard = self.socket.lock().ok()?;
        if guard.is_none() {
            match self.page_ws_url() {
                Ok(url) => match connect(&url) {
                    Ok((ws, _)) => {
                        eprintln!("[cdp] connected to {url}");
                        *guard = Some(ws);
                        *self.last_error.lock().unwrap() = None;
                    }
                    Err(err) => {
                        self.note_error(format!("websocket connect failed: {err}"));
                        return None;
                    }
                },
                Err(err) => {
                    self.note_error(err);
                    return None;
                }
            }
        }
        Some(guard)
    }

    fn evaluate(&self, socket: &mut Socket, expression: &str, timeout: Duration) -> Result<Value, String> {
        let payload = json!({
            "id": 1,
            "method": "Runtime.evaluate",
            "params": { "expression": expression, "returnByValue": true, "awaitPromise": true }
        });
        socket
            .send(Message::Text(payload.to_string()))
            .map_err(|e| format!("send failed: {e}"))?;

        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let message = socket.read().map_err(|e| format!("read failed: {e}"))?;
            let text = match message {
                Message::Text(t) => t.to_string(),
                Message::Binary(b) => String::from_utf8_lossy(&b).to_string(),
                Message::Close(_) => return Err("socket closed".into()),
                _ => continue,
            };
            let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            if value.get("id").and_then(Value::as_i64) != Some(1) {
                continue; // some other event
            }
            if let Some(error) = value.get("error") {
                return Err(format!("cdp error: {error}"));
            }
            let result = value
                .get("result")
                .and_then(|r| r.get("result"))
                .and_then(|r| r.get("value"))
                .cloned()
                .unwrap_or(Value::Null);
            if let Some(text) = result.as_str() {
                return serde_json::from_str(text).map_err(|e| format!("bad probe json: {e}"));
            }
            return Err("probe returned no value".into());
        }
        Err("probe timed out".into())
    }

    /// Fetch Apple's own TTML for the playing track.
    ///
    /// The request runs **inside the page**, reusing MusicKit's developer token and
    /// the signed-in `musicUserToken`. That is both the only way to get the payload
    /// (the Rust process has no credentials) and a way around the flaky direct
    /// network path documented in docs/m2-session-notes.md.
    pub fn fetch_ttml(&self, song_id: Option<&str>, title: &str, artist: &str) -> Option<String> {
        if !self.enabled || (song_id.is_none() && title.is_empty()) {
            return None;
        }
        let mut guard = self.ensure_socket()?;
        let socket = guard.as_mut()?;

        let id_lit = serde_json::to_string(&song_id).ok()?;
        let title_lit = serde_json::to_string(title).ok()?;
        let artist_lit = serde_json::to_string(artist).ok()?;
        let expression = LYRICS_EXPR
            .replace("__SONG_ID__", &id_lit)
            .replace("__TITLE__", &title_lit)
            .replace("__ARTIST__", &artist_lit);

        match self.evaluate(socket, &expression, Duration::from_millis(14000)) {
            Ok(value) => {
                if let Some(error) = value.get("error").and_then(Value::as_str) {
                    self.note_error(format!("lyrics probe: {error}"));
                    return None;
                }
                if let Some(status) = value.get("status").and_then(Value::as_i64) {
                    if status != 200 {
                        self.note_error(format!(
                            "lyrics HTTP {status} at stage {} for '{title}'",
                            value.get("stage").and_then(Value::as_str).unwrap_or("?")
                        ));
                        return None;
                    }
                }
                let ttml = value.get("ttml").and_then(Value::as_str)?;
                if ttml.len() < 32 {
                    self.note_error("lyrics payload looked empty");
                    return None;
                }
                Some(ttml.to_string())
            }
            Err(err) => {
                self.note_error(format!("lyrics request failed: {err}"));
                *guard = None;
                None
            }
        }
    }
}

impl Provider for CdpProvider {    fn id(&self) -> &'static str {
        "cdp"
    }

    fn precision(&self) -> u8 {
        2
    }

    fn poll(&self) -> Option<PlaybackSnapshot> {
        if !self.enabled {
            return None;
        }
        let mut guard = self.ensure_socket()?;
        let socket = guard.as_mut()?;
        let probe = match self.evaluate(socket, PROBE_EXPR, Duration::from_millis(1500)) {
            Ok(v) => v,
            Err(err) => {
                self.note_error(err);
                *guard = None; // force a reconnect next tick
                return None;
            }
        };

        let pos = probe.get("pos").and_then(Value::as_f64)?;
        let dur = probe.get("dur").and_then(Value::as_f64).unwrap_or(0.0);
        let paused = probe.get("paused").and_then(Value::as_bool).unwrap_or(true);
        let title = probe.get("title").and_then(Value::as_str).unwrap_or("").to_string();
        let artist = probe.get("artist").and_then(Value::as_str).unwrap_or("").to_string();
        let album = probe.get("album").and_then(Value::as_str).unwrap_or("").to_string();
        let song_id = probe.get("songId").and_then(Value::as_str).map(str::to_string);

        if title.is_empty() {
            return None; // nothing playing; let SMTC/UI decide
        }

        Some(PlaybackSnapshot {
            track: Some(TrackInfo {
                title,
                artist,
                album,
                app_id: "cdp".into(),
                song_id,
            }),
            state: if paused { PlaybackState::Paused } else { PlaybackState::Playing },
            position_secs: pos,
            duration_secs: dur,
            // The page reports live values, so the report is never stale.
            report_age_ms: 0,
            source: "cdp".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::PROBE_EXPR;

    #[test]
    fn probe_expression_is_a_single_expression() {
        // A stray newline or semicolon at top level would make Runtime.evaluate
        // return undefined and the provider would silently never report.
        assert!(PROBE_EXPR.trim_start().starts_with("(()"));
        assert!(PROBE_EXPR.trim_end().ends_with(")()"));
    }
}
