//! Persisted settings: `%APPDATA%\lyric-float\config.json`, hand-editable on purpose
//! (decision #11) so hotkeys and styling can change without a rebuild.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub toggle_visible: String,
    pub toggle_edit: String,
    pub font_up: String,
    pub font_down: String,
    pub offset_earlier: String,
    pub offset_later: String,
    /// Window nudging while edit mode is on. The overlay is permanently click-through
    /// (clearing that style bit stops this window from compositing at all), so the
    /// position is adjusted from the keyboard instead of by dragging.
    pub move_left: String,
    pub move_right: String,
    pub move_up: String,
    pub move_down: String,
    /// Put the overlay back on the monitor under the cursor (multi-monitor rescue).
    pub reset_position: String,
    pub play_pause: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            toggle_visible: "Alt+Backquote".into(),
            toggle_edit: "Alt+M".into(),
            font_up: "Alt+BracketRight".into(),
            font_down: "Alt+BracketLeft".into(),
            offset_earlier: "Alt+Comma".into(),
            offset_later: "Alt+Period".into(),
            move_left: "Alt+Left".into(),
            move_right: "Alt+Right".into(),
            move_up: "Alt+Up".into(),
            move_down: "Alt+Down".into(),
            reset_position: "Alt+0".into(),
            play_pause: "Alt+P".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub font_size: u32,
    pub font_family: String,
    pub fill_color: String,
    pub base_color: String,
    pub stroke_width: u32,
    pub stroke_color: String,
    pub opacity: f32,

    pub window_x: i32,
    pub window_y: i32,
    pub window_width: u32,
    pub window_height: u32,

    pub offset_ms: i64,
    pub edit_grace_ms: u64,

    /// None = auto-rank sessions (see provider::rank).
    pub pinned_app_id: Option<String>,
    /// Allow the lrclib tier to go online.
    pub allow_network: bool,
    /// 0 = TTML only, 1 = lrclib (default), 2 = keep going into manual tier.
    pub lyrics_tier: u8,

    pub cdp_enabled: bool,
    pub cdp_port: u16,
    /// Start the debug browser ourselves when nothing is listening. This is what
    /// removes the "remember to run the script" step: the overlay opens, the browser
    /// comes up with it.
    pub cdp_autolaunch: bool,
    /// Explicit chrome.exe path; empty = look in the usual install locations.
    pub cdp_chrome_path: Option<String>,
    /// Chrome user-data dir for the overlay's dedicated profile. Empty = default
    /// (`%APPDATA%\lyric-float\chrome-cdp-profile`).
    pub cdp_profile_dir: Option<String>,
    pub cdp_url: String,

    /// Accept playback data from the LyricFloat Bridge browser extension.
    pub ext_enabled: bool,
    pub ext_port: u16,
    /// Shared secret in the WebSocket path (`ws://127.0.0.1:<port>/<token>`).
    pub ext_token: String,

    pub auto_start: bool,
    /// Override where WebView2 keeps its profile. Empty = Tauri default
    /// (`%LOCALAPPDATA%\<identifier>`). Some hardened Windows setups make that
    /// path unusable, in which case pointing this at a writable folder is the fix.
    pub webview_data_dir: Option<String>,
    pub hotkeys: Hotkeys,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font_size: 34,
            font_family: "\"Source Han Sans SC\", \"Microsoft YaHei\", sans-serif".into(),
            fill_color: "#7ce3ff".into(),
            base_color: "rgba(255,255,255,0.30)".into(),
            stroke_width: 2,
            stroke_color: "rgba(0,0,0,0.85)".into(),
            opacity: 0.85,
            window_x: 500,
            window_y: 760,
            window_width: 900,
            window_height: 200,
            offset_ms: 0,
            edit_grace_ms: 5000,
            pinned_app_id: None,
            allow_network: true,
            lyrics_tier: 1,
            cdp_enabled: false,
            cdp_port: 9222,
            cdp_autolaunch: true,
            cdp_chrome_path: None,
            cdp_profile_dir: None,
            cdp_url: "https://music.apple.com/".into(),
            ext_enabled: true,
            ext_port: 8787,
            ext_token: "lyricfloat".into(),
            auto_start: false,
            webview_data_dir: None,
            hotkeys: Hotkeys::default(),
        }
    }
}

pub fn config_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("lyric-float").join("config.json")
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|err| {
                eprintln!("[config] {} is invalid ({err}), using defaults", path.display());
                Config::default()
            }),
            Err(_) => {
                let cfg = Config::default();
                let _ = cfg.save();
                cfg
            }
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = config_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(path, text)
    }
}
