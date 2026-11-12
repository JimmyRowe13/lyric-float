//! Feed layer.
//!
//! The overlay never talks to a player directly: it polls `Provider`s and renders
//! whatever the best one currently reports. SMTC is the zero-config default; CDP
//! is the precision upgrade; a manual provider lets the user paste lyrics.

pub mod cdp;
pub mod extension;
pub mod smtc;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub app_id: String,
    /// Catalog id, when the provider can see it (CDP reads it off MusicKit).
    /// Needed to ask Apple for that song's TTML lyrics.
    #[serde(default)]
    pub song_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackState {
    Playing,
    Paused,
    Stopped,
    Unknown,
}

impl PlaybackState {
    pub fn is_playing(self) -> bool {
        matches!(self, PlaybackState::Playing)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackSnapshot {
    pub track: Option<TrackInfo>,
    pub state: PlaybackState,
    /// Seconds, already extrapolated to "now" (see docs/m0-smtc-probe.md).
    pub position_secs: f64,
    pub duration_secs: f64,
    /// How stale the underlying report was, in millis. The UI uses this to decide
    /// between "trust" and "show not-connected" instead of drifting silently.
    pub report_age_ms: u64,
    pub source: String,
}

impl PlaybackSnapshot {
    pub fn is_usable(&self) -> bool {
        self.track.is_some() && self.duration_secs > 0.0 && self.report_age_ms < 3000
    }
}

pub trait Provider: Send + Sync {
    /// Stable id, also used as `PlaybackSnapshot::source`.
    fn id(&self) -> &'static str;

    /// Best snapshot this provider can produce right now, or `None`.
    fn poll(&self) -> Option<PlaybackSnapshot>;

    /// 0 = line-level only, 1 = sub-100 ms, 2 = word-level capable.
    fn precision(&self) -> u8 { 0 }
}

/// Pick the session worth following.
///
/// M0 finding: `SourceAppUserModelId` looks like `Chrome.AAZS3A6TMWYPI7NBWIMNFUAHSQ` and
/// carries **no URL**, so a domain check is impossible here (see docs/decisions.md #8).
/// We therefore rank by: playing first, then freshest report, then duration > 0.
pub fn rank<'a, T>(
    candidates: impl IntoIterator<Item = (&'a T, PlaybackState, u64, f64)>,
) -> Option<&'a T> {
    let mut best: Option<(&'a T, u8, i64, u64)> = None;
    for (item, state, age_ms, duration) in candidates {
        let playing = if state.is_playing() { 1u8 } else { 0 };
        let has_duration = if duration > 0.0 { 1i64 } else { 0 };
        // Higher score wins: playing, then has duration, then fresher (invert age).
        let score = (playing, has_duration, -(age_ms as i64));
        let better = match best {
            None => true,
            Some((_, b_play, b_dur, b_age)) => {
                let b_score = (b_play, b_dur, -(b_age as i64));
                score > b_score
            }
        };
        if better {
            best = Some((item, playing, has_duration, age_ms));
        }
    }
    best.map(|(item, _, _, _)| item)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_playing_over_fresher_paused() {
        let a = "paused-but-fresh";
        let b = "playing-older";
        let chosen = rank(vec![
            (&a, PlaybackState::Paused, 50, 200.0),
            (&b, PlaybackState::Playing, 800, 200.0),
        ]);
        assert_eq!(chosen, Some(&"playing-older"));
    }

    #[test]
    fn within_same_state_prefers_fresher() {
        let a = "stale";
        let b = "fresh";
        let chosen = rank(vec![
            (&a, PlaybackState::Playing, 900, 200.0),
            (&b, PlaybackState::Playing, 100, 200.0),
        ]);
        assert_eq!(chosen, Some(&"fresh"));
    }
}
