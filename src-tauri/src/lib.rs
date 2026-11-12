//! LyricFloat — desktop lyrics overlay for Apple Music web.
//!
//! NOT COMPILE-VERIFIED: written on a machine without a Rust toolchain.
//! Milestone M1 starts with `cargo check` and fixing whatever the API churn broke.

mod config;
mod lyrics;
mod overlay;
mod provider;

use std::sync::Mutex;
use std::time::Duration;
use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use config::Config;
use lyrics::{LyricLevel, LyricSet, Query};
use overlay::Overlay;
use provider::{PlaybackSnapshot, Provider, TrackInfo};

/// Keep the debug browser alive without the user having to remember a script.
///
/// Chrome can only open its DevTools port at launch, so an already running browser can
/// never be attached to; starting ours is the only way to make this invisible.
fn spawn_browser_keeper(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let mut last_attempt: Option<std::time::Instant> = None;
        let mut failures = 0u32;
        loop {
            std::thread::sleep(Duration::from_secs(5));

            let cfg = app.state::<AppState>().cfg.lock().unwrap().clone();
            if !cfg.cdp_enabled || !cfg.cdp_autolaunch {
                continue;
            }
            if provider::cdp::browser_alive(cfg.cdp_port) {
                failures = 0;
                continue;
            }
            if let Some(at) = last_attempt {
                if at.elapsed() < Duration::from_secs(20) {
                    continue;
                }
            }
            last_attempt = Some(std::time::Instant::now());
            let launched = provider::cdp::launch_browser(
                cfg.cdp_port,
                cfg.cdp_chrome_path.as_deref(),
                cfg.cdp_profile_dir.as_deref(),
                &cfg.cdp_url,
            );
            if launched {
                failures = 0;
            } else {
                failures += 1;
                if failures >= 3 {
                    // Some environments (sandboxed parents, restricted jobs) refuse to
                    // start a browser from us at all; retrying forever just spams.
                    eprintln!(
                        "[cdp] giving up on auto-launch after {failures} attempts; use deploy\\launch.ps1"
                    );
                    break;
                }
            }
        }
    });
}

/// Turn Apple's TTML into the render model.
///
/// `itunes:timing="Line"` is common (this is what the web player gets for many
/// tracks); only `"Word"` means the spans carry their own timings.
fn apple_lyrics(xml: &str) -> LyricSet {
    let mut lines = lyrics::ttml::parse(xml, &["v1"]);
    lyrics::ttml::normalize(&mut lines, 5.0);
    let level = if xml.contains("itunes:timing=\"Word\"") {
        LyricLevel::Word
    } else {
        LyricLevel::Line
    };
    LyricSet { level, lines, offset_ms: 0, origin: "apple".into() }
}

pub struct AppState {
    pub cfg: Mutex<Config>,
    pub lyrics: Mutex<LyricSet>,
    /// Playback data pushed by the browser extension.
    pub ext_feed: provider::extension::SharedFeed,
}

#[tauri::command]
fn get_lyrics(state: tauri::State<'_, AppState>) -> LyricSet {
    state.lyrics.lock().unwrap().clone()
}

#[tauri::command]
fn set_edit_mode(app: tauri::AppHandle, enabled: bool) {
    let grace = grace_of(&app);
    if enabled {
        overlay::enter_edit(&app, grace);
    } else {
        overlay::leave_edit(&app);
    }
}

#[tauri::command]
fn start_drag(app: tauri::AppHandle) {
    overlay::begin_drag(&app);
}

#[tauri::command]
fn drag_to(app: tauri::AppHandle, dx: i32, dy: i32) {
    overlay::drag_by(&app, dx, dy);
}

#[tauri::command]
fn end_drag(app: tauri::AppHandle) {
    // Persist the new position once the user lets go.
    let Some(window) = app.get_webview_window("lyrics") else { return };
    overlay::save_geometry(&app, &window);
}

#[tauri::command]
fn nudge_offset(app: tauri::AppHandle, delta_ms: i64) {
    let state = app.state::<AppState>();
    let mut cfg = state.cfg.lock().unwrap().clone();
    cfg.offset_ms = (cfg.offset_ms + delta_ms).clamp(-30_000, 30_000);
    let _ = cfg.save();
    *state.cfg.lock().unwrap() = cfg.clone();

    let shifted = {
        let mut current = state.lyrics.lock().unwrap();
        lyrics::lrc::apply_offset(&mut current.lines, delta_ms);
        current.offset_ms = cfg.offset_ms;
        current.clone()
    };
    let _ = app.emit("lyrics://set", &shifted);
    let _ = app.emit("overlay://hint", format!("歌词偏移 {:+.1}s", cfg.offset_ms as f64 / 1000.0));
}

#[tauri::command]
fn nudge_font(app: tauri::AppHandle, delta_px: i64) {
    let state = app.state::<AppState>();
    let mut cfg = state.cfg.lock().unwrap().clone();
    cfg.font_size = (cfg.font_size as i64 + delta_px).clamp(14, 96) as u32;
    let _ = cfg.save();
    *state.cfg.lock().unwrap() = cfg.clone();
    let _ = app.emit("overlay://hint", format!("字号 {}px", cfg.font_size));
}

fn grace_of(app: &tauri::AppHandle) -> Duration {
    let ms = app.state::<AppState>().cfg.lock().unwrap().edit_grace_ms;
    Duration::from_millis(ms.max(500))
}

/// `Code` follows the WebIDL-style naming (`KeyL`, `Digit1`, `ArrowUp`), so a
/// config written as "Alt+L" needs normalising before it can be parsed.
fn code_from_token(token: &str) -> Option<Code> {
    let t = token.trim();
    if t.len() == 1 {
        let c = t.chars().next()?;
        if c.is_ascii_alphabetic() {
            return format!("Key{}", c.to_ascii_uppercase()).parse::<Code>().ok();
        }
        if c.is_ascii_digit() {
            return format!("Digit{c}").parse::<Code>().ok();
        }
    }
    let canonical = match t {
        "Return" => "Enter",
        "Esc" => "Escape",
        "Up" => "ArrowUp",
        "Down" => "ArrowDown",
        "Left" => "ArrowLeft",
        "Right" => "ArrowRight",
        "`" => "Backquote",
        "[" => "BracketLeft",
        "]" => "BracketRight",
        "," => "Comma",
        "." => "Period",
        "/" => "Slash",
        ";" => "Semicolon",
        "'" => "Quote",
        "-" => "Minus",
        "=" => "Equal",
        other => other,
    };
    canonical.parse::<Code>().ok()
}

fn parse_shortcut(spec: &str) -> Option<Shortcut> {
    let mut mods = Modifiers::empty();
    let mut code = None;
    for part in spec.split('+') {
        match part.trim() {
            "Alt" => mods |= Modifiers::ALT,
            "Ctrl" | "Control" => mods |= Modifiers::CONTROL,
            "Shift" => mods |= Modifiers::SHIFT,
            "Win" | "Super" => mods |= Modifiers::META,
            other => {
                code = code_from_token(other);
            }
        }
    }
    Some(Shortcut::new(Some(mods), code?))
}

/// Bind one global shortcut. Returns false (and logs) instead of aborting the rest:
/// a single key already taken by another app must not knock out every hotkey.
fn bind(
    app: &tauri::AppHandle,
    spec: &str,
    action: impl Fn(&tauri::AppHandle) + Send + Sync + 'static,
) -> bool {
    let Some(sc) = parse_shortcut(spec) else {
        eprintln!("[hotkeys] cannot parse '{spec}'");
        return false;
    };
    let gs = app.global_shortcut();
    match gs.on_shortcut(sc, move |handle, _, event| {
        if event.state() == ShortcutState::Pressed {
            action(handle);
        }
    }) {
        Ok(()) => {
            println!("[hotkeys] bound {spec}");
            true
        }
        Err(err) => {
            eprintln!("[hotkeys] '{spec}' unavailable ({err}), trying next candidate");
            false
        }
    }
}

/// Bind the first candidate that the OS accepts, so one occupied key does not
/// leave a feature unreachable.
fn bind_first(
    app: &tauri::AppHandle,
    candidates: &[String],
    action: impl Fn(&tauri::AppHandle) + Send + Sync + 'static + Clone,
) -> Option<String> {
    for spec in candidates {
        if bind(app, spec, action.clone()) {
            return Some(spec.clone());
        }
    }
    eprintln!("[hotkeys] no candidate worked for {:?}", candidates);
    None
}

fn register_hotkeys(app: &tauri::AppHandle) {
    let cfg = app.state::<AppState>().cfg.lock().unwrap().hotkeys.clone();

    // Visible / hidden. Falls back through a few low-conflict combos because
    // `Alt+\`` is commonly taken (it was on the development machine).
    let toggle_visible = vec![
        cfg.toggle_visible.clone(),
        "Alt+L".to_string(),
        "Ctrl+Alt+L".to_string(),
        "Alt+Shift+L".to_string(),
    ];
    bind_first(app, &toggle_visible, |handle| {
        let h = handle.clone();
        // Window styles belong to the UI thread. Touching them from the global-hotkey
        // callback thread destroyed the WebView2 composition surface (the overlay went
        // blank until the next style change), so every window action is marshalled.
        let _ = handle.run_on_main_thread(move || {
            if let Some(w) = h.get_webview_window("lyrics") {
                let visible = w.is_visible().unwrap_or(true);
                let _ = if visible { w.hide() } else { w.show() };
            }
        });
    });

    let toggle_edit = vec![cfg.toggle_edit.clone(), "Alt+Shift+E".to_string()];
    bind_first(app, &toggle_edit, |handle| {
        let h = handle.clone();
        let _ = handle.run_on_main_thread(move || overlay::toggle_edit(&h, grace_of(&h)));
    });

    // The rest are plain bindings; failures are logged and ignored.
    let font_up = cfg.font_up.clone();
    bind(app, &font_up, |handle| nudge_font(handle.clone(), 2));
    let font_down = cfg.font_down.clone();
    bind(app, &font_down, |handle| nudge_font(handle.clone(), -2));
    let earlier = cfg.offset_earlier.clone();
    bind(app, &earlier, |handle| nudge_offset(handle.clone(), -100));
    let later = cfg.offset_later.clone();
    bind(app, &later, |handle| nudge_offset(handle.clone(), 100));

    // Position nudging. The overlay never takes mouse input (it must stay click-through
    // to keep rendering), so arrows replace dragging; they only act in edit mode.
    let moves: [(String, i32, i32); 4] = [
        (cfg.move_left.clone(), -12, 0),
        (cfg.move_right.clone(), 12, 0),
        (cfg.move_up.clone(), 0, -12),
        (cfg.move_down.clone(), 0, 12),
    ];
    for (spec, dx, dy) in moves {
        bind(app, &spec, move |handle| {
            if !overlay::is_editing(handle) {
                return;
            }
            let h = handle.clone();
            let _ = handle.run_on_main_thread(move || overlay::nudge_position(&h, dx, dy));
        });
    }

    // Multi-monitor rescue: after a monitor is unplugged or rearranged the saved
    // position can end up outside the desktop, and the overlay would be unreachable.
    let reset = cfg.reset_position.clone();
    bind(app, &reset, |handle| {
        let h = handle.clone();
        let _ = handle.run_on_main_thread(move || overlay::reset_position(&h));
    });
}

/// Poll the feed layer and push snapshots to the overlay.
fn spawn_poller(app: tauri::AppHandle) {
    let smtc = provider::smtc::SmtcProvider::new();
    let cdp = {
        let cfg = app.state::<AppState>().cfg.lock().unwrap().clone();
        provider::cdp::CdpProvider::new(cfg.cdp_port, cfg.cdp_enabled)
    };
    let ext = {
        let cfg = app.state::<AppState>().cfg.lock().unwrap().clone();
        provider::extension::ExtensionProvider::new(app.state::<AppState>().ext_feed.clone(), cfg.ext_enabled)
    };
    let mut last_track: Option<TrackInfo> = None;
    let mut last_emit = std::time::Instant::now();
    let mut last_lyric_seq: u64 = 0;

    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(200));

        // Re-pin if the user picked a session from the tray.
        let pinned = app.state::<AppState>().cfg.lock().unwrap().pinned_app_id.clone();
        smtc.pin(pinned);

        // Feed priority: the extension talks to the page directly (no debug port, no
        // dedicated browser), CDP gives the same data when a debug browser is used,
        // and SMTC is the zero-config fallback.
        let Some(snap) = ext.poll().or_else(|| cdp.poll()).or_else(|| smtc.poll()) else {
            continue;
        };

        let changed = match (&snap.track, &last_track) {
            (Some(a), Some(b)) => a.title != b.title || a.artist != b.artist,
            (Some(_), None) => true,
            _ => false,
        };

        // Emit at ~5 Hz; the frontend runs its own clock in between.
        if last_emit.elapsed() >= Duration::from_millis(200) {
            let _ = app.emit("player://snapshot", &snap);
            last_emit = std::time::Instant::now();
        }

        // Lyrics pushed by the extension arrive out of band, keyed by a sequence number.
        let pending = {
            // Keep the managed state alive: the mutex guard borrows from it.
            let state = app.state::<AppState>();
            let guard = state.ext_feed.lock().unwrap();
            match (&guard.lyrics, guard.lyric_seq) {
                (Some((id, ttml)), seq) if seq != last_lyric_seq => {
                    Some((id.clone(), ttml.clone(), seq))
                }
                _ => None,
            }
        };
        if let Some((_song_id, ttml, seq)) = pending {
            last_lyric_seq = seq;
            let mut set = apple_lyrics(&ttml);
            let offset = app.state::<AppState>().cfg.lock().unwrap().offset_ms;
            set.offset_ms = offset;
            println!(
                "[poller] lyrics from extension: origin={} level={:?} lines={}",
                set.origin,
                set.level,
                set.lines.len()
            );
            *app.state::<AppState>().lyrics.lock().unwrap() = set.clone();
            let _ = app.emit("lyrics://set", &set);
        }

        if changed {
            last_track = snap.track.clone();
            if let Some(track) = snap.track.clone() {
                println!("[poller] track changed: {} - {} ({:.1}s)", track.title, track.artist, snap.duration_secs);
                let handle = app.clone();
                let port = app.state::<AppState>().cfg.lock().unwrap().cdp_port;
                let cdp_on = app.state::<AppState>().cfg.lock().unwrap().cdp_enabled;
                let duration = if snap.duration_secs > 0.0 { snap.duration_secs } else { track_duration(&snap) };
                std::thread::spawn(move || {
                    let query = Query {
                        title: track.title.clone(),
                        artist: track.artist.clone(),
                        album: track.album.clone(),
                        duration_secs: duration,
                        app_id: track.app_id.clone(),
                    };
                    let allow_network = handle.state::<AppState>().cfg.lock().unwrap().allow_network;

                    // Tier 1: the player's own TTML. A dedicated provider instance so a
                    // slow lyric request cannot block the 5 Hz position poll. When the
                    // extension is the feed it already pushed the TTML over its socket,
                    // so do not ask again.
                    let apple = if cdp_on && snap.source.as_str() != "extension" {
                        let fetcher = provider::cdp::CdpProvider::new(port, true);
                        fetcher.fetch_ttml(track.song_id.as_deref(), &track.title, &track.artist)
                    } else {
                        None
                    };

                    let mut set = match apple {
                        Some(xml) => apple_lyrics(&xml),
                        None => lyrics::resolve_automatic(&query, allow_network),
                    };

                    println!(
                        "[poller] lyrics resolved: origin={} level={:?} lines={}",
                        set.origin,
                        set.level,
                        set.lines.len()
                    );
                    let offset = handle.state::<AppState>().cfg.lock().unwrap().offset_ms;
                    set.offset_ms = offset;
                    *handle.state::<AppState>().lyrics.lock().unwrap() = set.clone();
                    let _ = handle.emit("lyrics://set", &set);
                });
            } else {
                let _ = app.emit("lyrics://set", LyricSet::empty());
            }
        }
    });
}

fn track_duration(snap: &PlaybackSnapshot) -> f64 {
    snap.duration_secs
}

pub fn run() {
    let cfg = Config::load();

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(AppState {
            cfg: Mutex::new(cfg),
            lyrics: Mutex::new(LyricSet::empty()),
            ext_feed: provider::extension::new_feed(),
        })
        .manage(Overlay::new())
        .invoke_handler(tauri::generate_handler![
            get_lyrics,
            set_edit_mode,
            start_drag,
            drag_to,
            end_drag,
            nudge_offset,
            nudge_font
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let cfg = app.state::<AppState>().cfg.lock().unwrap().clone();

            // The window is built here, not from tauri.conf.json, because Tauri
            // forces WebView2's data directory to app_local_data_dir on Windows.
            // On machines where that path is unusable the webview dies with
            // E_UNEXPECTED before it renders anything, so we keep it overridable.
            let data_dir = match cfg.webview_data_dir.as_deref().map(str::trim) {
                Some(p) if !p.is_empty() => Some(std::path::PathBuf::from(p)),
                _ => app.path().app_local_data_dir().ok(),
            };

            let mut builder = tauri::WebviewWindowBuilder::new(
                app,
                "lyrics",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("LyricFloat")
            .inner_size(cfg.window_width as f64, cfg.window_height as f64)
            .position(cfg.window_x as f64, cfg.window_y as f64)
            .transparent(true)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .shadow(false)
            .focused(false);

            if let Some(dir) = data_dir {
                builder = builder.data_directory(dir);
            }

            // Debug builds expose the webview to the DevTools protocol so the
            // overlay's own DOM can be inspected: http://127.0.0.1:9333/json
            #[cfg(debug_assertions)]
            {
                builder = builder.additional_browser_args("--remote-debugging-port=9333");
            }

            let window = builder.build()?;
            {
                // A saved position may no longer exist (monitor unplugged / rearranged).
                // Deferred: right after `build()` the window is not mapped yet, so its
                // geometry reads back as an error or as 0,0.
                let h = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(600));
                    let inner = h.clone();
                    let _ = h.run_on_main_thread(move || overlay::ensure_on_screen(&inner));
                });
            }
            overlay::set_passthrough(&window, true);
            overlay::spawn_passthrough_guard(handle.clone());

            register_hotkeys(&handle);
            {
                // Local WebSocket server for the browser extension. Loopback only.
                let cfg = app.state::<AppState>().cfg.lock().unwrap().clone();
                if cfg.ext_enabled {
                    provider::extension::start_server(
                        cfg.ext_port,
                        cfg.ext_token.clone(),
                        app.state::<AppState>().ext_feed.clone(),
                    );
                }
            }
            spawn_browser_keeper(handle.clone());
            spawn_poller(handle);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running LyricFloat");
}
