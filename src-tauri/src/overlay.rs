//! Window behaviour: click-through, edit mode with a grace period, dragging.
//!
//! Decision #4: the overlay is click-through by default. A hotkey flips it into
//! "edit" mode for `edit_grace_ms`, then it goes back by itself, because a floating
//! bar you forgot to unlock is worse than one you have to unlock again.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager, PhysicalPosition, WebviewWindow};

pub struct EditState {
    pub editing: bool,
    pub deadline: Option<Instant>,
    pub drag_origin: Option<(i32, i32)>,
}

impl Default for EditState {
    fn default() -> Self {
        Self { editing: false, deadline: None, drag_origin: None }
    }
}

pub struct Overlay {
    pub state: Mutex<EditState>,
    /// The state we want the *whole window tree* to be in, so the guard thread can
    /// restore it after WebView2 recreates its child windows.
    pub passthrough: AtomicBool,
}

impl Overlay {
    pub fn new() -> Self {
        Self { state: Mutex::new(EditState::default()), passthrough: AtomicBool::new(true) }
    }
}

/// Direct user32 FFI on purpose: Tauri's `HWND` comes from a different `windows`
/// crate version than ours, so `window.hwnd()` cannot be passed across without
/// dragging in a version bump. `.0 as isize` works either way.
#[cfg(windows)]
mod win {
    pub const GWL_EXSTYLE: i32 = -20;
    pub const WS_EX_TRANSPARENT: isize = 0x0000_0020;
    pub const WS_EX_LAYERED: isize = 0x0008_0000;
    pub const LWA_ALPHA: u32 = 0x0000_0002;

    /// `WS_EX_TRANSPARENT` only removes a window from hit-testing while the window is
    /// also `WS_EX_LAYERED`; without the layered bit it is just a painting hint and
    /// clicks keep landing on the overlay. Earlier code went through tao's
    /// `set_ignore_cursor_events`, which recomputes the style set and drops the layered
    /// bit Tauri needs, so it is asserted here instead of assumed.
    pub unsafe fn ensure_layered(hwnd: isize) {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if ex & WS_EX_LAYERED == 0 {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_LAYERED);
            // A layered window with no layered attributes is not drawn at all.
            SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
        }
    }

    #[link(name = "user32")]
    extern "system" {
        pub fn EnumChildWindows(
            parent: isize,
            callback: Option<unsafe extern "system" fn(isize, isize) -> i32>,
            lparam: isize,
        ) -> i32;
        pub fn GetWindowLongPtrW(hwnd: isize, index: i32) -> isize;
        pub fn SetWindowLongPtrW(hwnd: isize, index: i32, value: isize) -> isize;
        pub fn SetLayeredWindowAttributes(hwnd: isize, key: u32, alpha: u8, flags: u32) -> i32;
        pub fn GetCursorPos(point: *mut Point) -> i32;
    }

    #[repr(C)]
    #[derive(Default)]
    pub struct Point {
        pub x: i32,
        pub y: i32,
    }

    /// Cursor position in virtual-screen coordinates — the same space the monitor
    /// rectangles use, so coordinates are negative on a monitor placed left of the
    /// primary one.
    pub unsafe fn cursor_position() -> Option<(i32, i32)> {
        let mut point = Point::default();
        if GetCursorPos(&mut point) != 0 {
            Some((point.x, point.y))
        } else {
            None
        }
    }

    /// Flip the click-through bit. Deliberately no SetWindowPos(SWP_FRAMECHANGED) here:
    /// doing that on WebView2's Chromium child windows destroys their composition
    /// surface, and the overlay renders nothing until the bit is flipped back —
    /// which looked exactly like "pressing Alt+M hides the lyrics".
    pub unsafe extern "system" fn apply_to_child(hwnd: isize, lparam: isize) -> i32 {
        let want = lparam != 0;
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let next = if want { ex | WS_EX_TRANSPARENT } else { ex & !WS_EX_TRANSPARENT };
        if next != ex {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next);
        }
        1
    }
}

/// Apply click-through to the whole window tree.
///
/// Setting `WS_EX_TRANSPARENT` on the top-level window is not enough on Windows:
/// WebView2 hosts its content in **child** windows, which keep hit-testing normally
/// and swallow clicks. Measured with `WindowFromPoint` before this fix: points inside
/// the overlay resolved to a child HWND of our own process, i.e. clicks never reached
/// the browser underneath.
pub fn set_passthrough(window: &WebviewWindow, passthrough: bool) {
    #[cfg(windows)]
    {
        if let Ok(hwnd) = window.hwnd() {
            let raw: isize = hwnd.0 as isize;
            let want = if passthrough { 1 } else { 0 };
            unsafe {
                // Deliberately NOT tao's `set_ignore_cursor_events()`: it recomputes the
                // whole extended style from its own flag set, which does not include the
                // WS_EX_LAYERED that click-through needs (and that Tauri uses for window
                // transparency). Flipping the bit through that path also cleared LAYERED,
                // which silently turned the overlay back into a click magnet.
                win::ensure_layered(raw);
                win::apply_to_child(raw, want);
                win::EnumChildWindows(raw, Some(win::apply_to_child), want);
            }
        }
    }
    #[cfg(not(windows))]
    {
        if let Err(err) = window.set_ignore_cursor_events(passthrough) {
            eprintln!("[overlay] set_ignore_cursor_events failed: {err}");
        }
    }
}

/// WebView2 rebuilds its child windows on navigation/reload, which silently drops our
/// style bits, so re-assert the wanted state a few times per second.
pub fn spawn_passthrough_guard(app: tauri::AppHandle) {
    std::thread::spawn(move || loop {
        // WebView2 creates and destroys child windows whenever it navigates or swaps its
        // rendering surface. A newly created child starts *without* the click-through
        // bit, and during that gap clicks land on the overlay instead of the window
        // underneath, which feels like "this spot is not click-through". 120 ms keeps
        // the gap below a human click, and the style write is a no-op when it already
        // matches, so the cost is one EnumChildWindows per tick.
        std::thread::sleep(Duration::from_millis(120));
        let Some(window) = app.get_webview_window("lyrics") else { continue };
        let want = app.state::<Overlay>().passthrough.load(Ordering::Relaxed);
        set_passthrough(&window, want);
    });
}

/// Apply click-through to the overlay window.

pub fn enter_edit(app: &tauri::AppHandle, grace: Duration) {
    let Some(window) = app.get_webview_window("lyrics") else { return };
    let overlay = app.state::<Overlay>();
    {
        let mut st = overlay.state.lock().unwrap();
        st.editing = true;
        st.deadline = Some(Instant::now() + grace);
    }
    // The overlay stays click-through in every state. Clearing WS_EX_TRANSPARENT makes
    // this window stop presenting its WebView2 surface entirely (verified: the page
    // renders red, the screen shows nothing), so mouse input is not an option here.
    let _ = window.set_focus();
    let _ = app.emit("overlay://edit-mode", true);

    // Watchdog: leave edit mode when the deadline passes.
    let app_handle = app.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(200));
            let overlay = app_handle.state::<Overlay>();
            let expired = {
                let st = overlay.state.lock().unwrap();
                match st.deadline {
                    Some(d) => Instant::now() >= d,
                    None => !st.editing,
                }
            };
            if expired {
                leave_edit(&app_handle);
                break;
            }
        }
    });
}

pub fn leave_edit(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("lyrics") else { return };
    let overlay = app.state::<Overlay>();
    {
        let mut st = overlay.state.lock().unwrap();
        if !st.editing {
            return;
        }
        st.editing = false;
        st.deadline = None;
        st.drag_origin = None;
    }
    overlay.passthrough.store(true, Ordering::Relaxed);
    let _ = app.emit("overlay://edit-mode", false);
}

/// Move the overlay by (dx, dy) logical pixels and persist the position.
pub fn nudge_position(app: &tauri::AppHandle, dx: i32, dy: i32) {
    use tauri::Manager;
    let Some(window) = app.get_webview_window("lyrics") else { return };
    let Ok(pos) = window.outer_position() else { return };
    let scale = window.scale_factor().unwrap_or(1.0);
    let x = pos.x + (dx as f64 * scale) as i32;
    let y = pos.y + (dy as f64 * scale) as i32;
    let _ = window.set_position(tauri::Position::Physical(PhysicalPosition::new(x, y)));
    save_geometry(app, &window);
}

/// Persist the window geometry in **logical** units.
///
/// The builder takes logical points while `outer_position()`/`outer_size()` return
/// physical pixels. Storing the raw values made the overlay drift a little on every
/// restart on a scaled display, and jump outright whenever the monitor scale changed.
pub fn save_geometry(app: &tauri::AppHandle, window: &WebviewWindow) {
    let scale = window.scale_factor().unwrap_or(1.0);
    let Ok(pos) = window.outer_position() else { return };
    let Ok(size) = window.outer_size() else { return };
    let logical_pos = pos.to_logical::<f64>(scale);
    let logical_size = size.to_logical::<f64>(scale);

    let state = app.state::<crate::AppState>();
    let mut cfg = state.cfg.lock().unwrap();
    cfg.window_x = logical_pos.x.round() as i32;
    cfg.window_y = logical_pos.y.round() as i32;
    cfg.window_width = logical_size.width.round() as u32;
    cfg.window_height = logical_size.height.round() as u32;
    if let Err(err) = cfg.save() {
        eprintln!("[overlay] cannot save geometry: {err}");
    }
}

/// Put the overlay back on a monitor after the desktop layout changed (a monitor was
/// unplugged, rearranged, or the resolution changed). Without this the saved position
/// can point at coordinates that no longer exist, and the overlay is simply never seen.
pub fn ensure_on_screen(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("lyrics") else { return };
    let Ok(pos) = window.outer_position() else { return };
    let Ok(size) = window.outer_size() else { return };
    let monitors = window.available_monitors().unwrap_or_default();
    if monitors.is_empty() {
        return;
    }

    let center = (pos.x + size.width as i32 / 2, pos.y + size.height as i32 / 2);
    let on_screen = monitors.iter().any(|m| {
        let mp = m.position();
        let ms = m.size();
        center.0 >= mp.x
            && center.0 < mp.x + ms.width as i32
            && center.1 >= mp.y
            && center.1 < mp.y + ms.height as i32
    });

    if on_screen {
        // Windows may already have clamped an impossible position on creation; rewrite
        // the config so it stops carrying stale coordinates.
        save_geometry(app, &window);
        return;
    }

    if let Some(monitor) = window.primary_monitor().ok().flatten() {
        place_on(app, &window, &monitor, size);
    }
}

/// Centre the overlay on the monitor that currently holds the cursor.
pub fn reset_position(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("lyrics") else { return };
    let Ok(size) = window.outer_size() else { return };
    let monitors = window.available_monitors().unwrap_or_default();
    if monitors.is_empty() {
        return;
    }

    let cursor = unsafe { win::cursor_position() };
    let target = cursor
        .and_then(|(cx, cy)| {
            monitors
                .iter()
                .find(|m| {
                    let mp = m.position();
                    let ms = m.size();
                    cx >= mp.x && cx < mp.x + ms.width as i32 && cy >= mp.y && cy < mp.y + ms.height as i32
                })
                .cloned()
        })
        .or_else(|| window.primary_monitor().ok().flatten());

    if let Some(monitor) = target {
        place_on(app, &window, &monitor, size);
        let _ = app.emit("overlay://hint", "位置已重置到鼠标所在屏幕");
    }
}

fn place_on(
    app: &tauri::AppHandle,
    window: &WebviewWindow,
    monitor: &tauri::Monitor,
    size: tauri::PhysicalSize<u32>,
) {
    let mp = monitor.position();
    let ms = monitor.size();
    let x = mp.x + (ms.width as i32 - size.width as i32) / 2;
    let y = mp.y + 80;
    let _ = window.set_position(tauri::Position::Physical(PhysicalPosition::new(x, y)));
    save_geometry(app, window);
    println!(
        "[overlay] placed on {} at {x},{y}",
        monitor.name().map(String::as_str).unwrap_or("monitor")
    );
}

pub fn toggle_edit(app: &tauri::AppHandle, grace: Duration) {
    let editing = app.state::<Overlay>().state.lock().unwrap().editing;
    if editing {
        leave_edit(app);
    } else {
        enter_edit(app, grace);
    }
}

/// True while the overlay is in adjustment mode (arrows move it, outline is drawn).
pub fn is_editing(app: &tauri::AppHandle) -> bool {
    app.state::<Overlay>().state.lock().unwrap().editing
}

/// Called from the UI on mousedown while in edit mode: remember where we started.
pub fn begin_drag(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("lyrics") else { return };
    let origin = window.outer_position().ok().map(|p| (p.x, p.y));
    let overlay = app.state::<Overlay>();
    let mut st = overlay.state.lock().unwrap();
    st.drag_origin = origin;
    // Any drag interaction resets the grace timer.
    if st.editing {
        st.deadline = Some(Instant::now() + Duration::from_millis(5000));
    }
}

pub fn drag_by(app: &tauri::AppHandle, dx: i32, dy: i32) {
    let Some(window) = app.get_webview_window("lyrics") else { return };
    let overlay = app.state::<Overlay>();
    let origin = {
        let st = overlay.state.lock().unwrap();
        st.drag_origin
    };
    if let Some((ox, oy)) = origin {
        let _ = window.set_position(tauri::Position::Physical(PhysicalPosition::new(ox + dx, oy + dy)));
    }
}
