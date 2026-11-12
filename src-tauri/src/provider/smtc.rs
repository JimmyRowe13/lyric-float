//! Windows System Media Transport Controls provider.
//!
//! Verified behaviour (docs/m0-smtc-probe.md):
//!   * metadata, playback status and duration always work;
//!   * position is only trustworthy while the page keeps publishing it —
//!     a leftover session can keep a 25-hour-old `LastUpdatedTime` forever;
//!   * extrapolating by `report_age` overshoots when the report is fresh
//!     (quantisation ~250 ms), so we only extrapolate past a threshold.
//!
//! NOTE: written without a local Rust toolchain; expected to need small API
//! fixes on first `cargo check` (windows-rs naming churn).

use super::{PlaybackState, PlaybackSnapshot, Provider, TrackInfo};

/// Only follow browser sessions; a native music app is a different product.
const BROWSER_PREFIXES: &[&str] = &["Chrome.", "MsEdge.", "Firefox", "Brave.", "Opera."];

/// A paused/stopped session keeps its last timeline forever; that is fine because
/// a frozen position is the correct one. While playing, the age of the report is
/// elapsed playback time (see the M0/M2 notes), so we always extrapolate.
const EXTRAPOLATE_AFTER_MS: u64 = 0;

pub struct SmtcProvider {
    followed: std::sync::Mutex<Option<String>>,
}

impl Default for SmtcProvider {
    fn default() -> Self {
        Self { followed: std::sync::Mutex::new(None) }
    }
}

impl SmtcProvider {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pin to a specific app id (tray menu), or `None` to auto-rank.
    pub fn pin(&self, app_id: Option<String>) {
        *self.followed.lock().unwrap() = app_id;
    }
}

impl Provider for SmtcProvider {
    fn id(&self) -> &'static str {
        "smtc"
    }

    fn precision(&self) -> u8 {
        0
    }

    fn poll(&self) -> Option<PlaybackSnapshot> {
        platform::poll(self.followed.lock().unwrap().clone())
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use windows::core::HSTRING;
    use windows::Foundation::DateTime;
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSession as Session,
        GlobalSystemMediaTransportControlsSessionManager as SessionManager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
    };

    /// 100-ns ticks since 1601-01-01, the epoch WinRT uses.
    fn now_ticks() -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        let unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        unix / 100 + 116_444_736_000_000_000
    }

    fn ticks_to_secs(ts: windows::Foundation::TimeSpan) -> f64 {
        ts.Duration as f64 / 10_000_000.0
    }

    fn hstring(h: HSTRING) -> String {
        h.to_string()
    }

    struct Candidate {
        session: Session,
        app_id: String,
        state: PlaybackState,
        age_ms: u64,
        duration: f64,
        track: Option<TrackInfo>,
        position: f64,
        last_updated: DateTime,
    }

    fn describe(session: &Session) -> Option<Candidate> {
        let app_id = hstring(session.SourceAppUserModelId().ok()?);
        if !BROWSER_PREFIXES.iter().any(|p| app_id.starts_with(p)) {
            return None;
        }

        let info = session.GetPlaybackInfo().ok()?;
        let state = match info.PlaybackStatus().ok()? {
            Status::Playing => PlaybackState::Playing,
            Status::Paused => PlaybackState::Paused,
            Status::Stopped | Status::Closed => PlaybackState::Stopped,
            _ => PlaybackState::Unknown,
        };

        let track = session
            .TryGetMediaPropertiesAsync()
            .ok()
            .and_then(|op| op.get().ok())
            .and_then(|props| {
                let title = hstring(props.Title().ok()?);
                if title.is_empty() {
                    return None;
                }
                Some(TrackInfo {
                    title,
                    artist: hstring(props.Artist().unwrap_or_default()),
                    album: hstring(props.AlbumTitle().unwrap_or_default()),
                    app_id: app_id.clone(),
                    song_id: None, // SMTC exposes no catalog id
                })
            });

        let tl = session.GetTimelineProperties().ok()?;
        let last_updated = tl.LastUpdatedTime().ok()?;
        let position = ticks_to_secs(tl.Position().ok()?);
        let duration = ticks_to_secs(tl.EndTime().ok()?);
        let age_ticks = now_ticks().saturating_sub(last_updated.UniversalTime);
        let age_ms = (age_ticks / 10_000).max(0) as u64;

        Some(Candidate {
            session: session.clone(),
            app_id,
            state,
            age_ms,
            duration,
            track,
            position,
            last_updated,
        })
    }

    pub fn poll(pinned: Option<String>) -> Option<PlaybackSnapshot> {
        let manager: SessionManager = SessionManager::RequestAsync().ok()?.get().ok()?;
        let sessions = manager.GetSessions().ok()?;

        let mut candidates: Vec<Candidate> = Vec::new();
        for i in 0..sessions.Size().unwrap_or(0) {
            if let Ok(session) = sessions.GetAt(i) {
                if let Some(c) = describe(&session) {
                    candidates.push(c);
                }
            }
        }

        let chosen = if let Some(id) = pinned {
            candidates.into_iter().find(|c| c.app_id == id)
        } else {
            let scored: Vec<(&Candidate, PlaybackState, u64, f64)> =
                candidates.iter().map(|c| (c, c.state, c.age_ms, c.duration)).collect();
            let picked = super::super::rank(scored)?;
            // rank() borrows, so re-find by app id + index.
            let id = picked.app_id.clone();
            let mut found = None;
            for c in candidates.into_iter() {
                if c.app_id == id {
                    found = Some(c);
                    break;
                }
            }
            found
        }?;

        let mut position = chosen.position;
        if chosen.state.is_playing() && chosen.age_ms > EXTRAPOLATE_AFTER_MS {
            // Apple Music web publishes the timeline once and never refreshes it, so
            // `lastUpdated` is our anchor and the age is elapsed playback time.
            position += chosen.age_ms as f64 / 1000.0;
        }
        if chosen.duration > 0.0 {
            position = position.clamp(0.0, chosen.duration);
        }

        Some(PlaybackSnapshot {
            track: chosen.track,
            state: chosen.state,
            position_secs: position,
            duration_secs: chosen.duration,
            report_age_ms: chosen.age_ms,
            source: "smtc".into(),
        })
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    pub fn poll(_pinned: Option<String>) -> Option<PlaybackSnapshot> {
        None
    }
}
