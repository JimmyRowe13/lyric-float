//! Lyrics model + the three-tier waterfall (Apple TTML -> lrclib -> manual).

pub mod lrc;
pub mod lrclib;
pub mod netease;
pub mod ttml;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WordToken {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LyricLine {
    pub start: f64,
    pub end: f64,
    pub text: String,
    /// Empty for line-level sources (LRC). The renderer then lights the whole line.
    pub words: Vec<WordToken>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LyricLevel {
    Word,
    Line,
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LyricSet {
    pub level: LyricLevel,
    pub lines: Vec<LyricLine>,
    pub offset_ms: i64,
    /// Which tier produced this: "ttml" | "lrclib" | "manual" | "none"
    pub origin: String,
}

impl LyricSet {
    pub fn empty() -> Self {
        Self { level: LyricLevel::None, lines: Vec::new(), offset_ms: 0, origin: "none".into() }
    }

    pub fn from_lrc(text: &str, origin: &str) -> Self {
        let lines = lrc::parse(text);
        Self {
            level: if lines.is_empty() { LyricLevel::None } else { LyricLevel::Line },
            lines,
            offset_ms: lrc::offset_ms(text),
            origin: origin.into(),
        }
    }

    /// Recompute `end` for lines that lack one (LRC gives starts only).
    pub fn normalize(&mut self, fallback_tail: f64) {
        for i in 0..self.lines.len() {
            let next_start = self.lines.get(i + 1).map(|l| l.start);
            let end = self.lines[i].end;
            if end <= self.lines[i].start {
                self.lines[i].end = next_start.unwrap_or(self.lines[i].start + fallback_tail);
            }
        }
    }
}

/// Everything needed to resolve a track, independent of the provider.
#[derive(Debug, Clone)]
pub struct Query {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_secs: f64,
    pub app_id: String,
}

/// Automatic tiers, in order. NetEase first because lrclib.net is behind Cloudflare
/// and intermittently unreachable from mainland networks; lrclib stays as the
/// fallback for tracks NetEase does not carry.
pub fn resolve_automatic(query: &Query, allow_network: bool) -> LyricSet {
    if !allow_network {
        return LyricSet::empty();
    }

    if let Some(text) = netease::fetch_synced(query) {
        if !text.trim().is_empty() {
            return LyricSet::from_lrc(&text, "netease");
        }
    }

    if let Some(text) = lrclib::fetch_synced(query) {
        if !text.trim().is_empty() {
            return LyricSet::from_lrc(&text, "lrclib");
        }
    }

    LyricSet::empty()
}
