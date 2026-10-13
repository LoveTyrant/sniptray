#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(clippy::missing_safety_doc)]

mod capture;
mod config;
mod history;
mod overlay;
mod snip;
mod thumb;
mod util;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{LazyLock, Mutex, OnceLock};

use base64::Engine;
use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::{ManagerExt, MacosLauncher};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

pub static OVERLAY_ACTIVE: AtomicBool = AtomicBool::new(false);
/// True while the Settings window is capturing a new hotkey. The global
/// shortcut stays registered, but the handler ignores presses, so pressing
/// the current hotkey to reassign it doesn't fire a snip.
static CAPTURING_HOTKEY: AtomicBool = AtomicBool::new(false);
static APP: OnceLock<AppHandle> = OnceLock::new();
/// Set when the configured hotkey couldn't be registered; shown in Settings
/// until a hotkey is registered successfully.
static HOTKEY_ERROR: Mutex<Option<String>> = Mutex::new(None);
static EDITOR_SEQ: AtomicU32 = AtomicU32::new(0);
static EDITOR_PATHS: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn start_snip() {
    if OVERLAY_ACTIVE.swap(true, Ordering::SeqCst) {
        return;
    }
    let ok = capture::capture_virtual_screen()
        .map(|shot| overlay::show(shot).is_ok())
        .unwrap_or(false);
    if !ok {
        OVERLAY_ACTIVE.store(false, Ordering::SeqCst);
    }
}

/// Snip must run on the main thread: the overlay window is created there so
/// its messages are pumped by tao's event loop.
pub fn snip_on_main_thread() {
    if let Some(app) = APP.get() {
        let _ = app.run_on_main_thread(start_snip);
    }
}

/// Snip after a delay, so menus and hover states can be opened first.
fn snip_after(secs: u64) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(secs));
        snip_on_main_thread();
    });
}

/// Snip the whole monitor under the cursor, without the selection overlay.
fn start_full_snip() {
    if OVERLAY_ACTIVE.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(shot) = capture::capture_virtual_screen() {
        let mut pt = windows::Win32::Foundation::POINT::default();
        let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt) };
        let p = windows::Win32::Foundation::POINT { x: pt.x - shot.x, y: pt.y - shot.y };
        let r = capture::monitor_rects(&shot)
            .into_iter()
            .find(|r| capture::rect_contains(r, p))
            .unwrap_or(windows::Win32::Foundation::RECT {
                left: 0,
                top: 0,
                right: shot.w,
                bottom: shot.h,
            });
        snip::complete(&shot, r);
    }
    OVERLAY_ACTIVE.store(false, Ordering::SeqCst);
}

fn full_snip_on_main_thread() {
    // Give the tray menu a moment to fade out so it isn't in the shot.
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(250));
        if let Some(app) = APP.get() {
            let _ = app.run_on_main_thread(start_full_snip);
        }
    });
}

/// Re-copy a recent snip and bring its preview back (to drag or edit).
fn open_recent(path: String) {
    let Ok(bytes) = std::fs::read(&path) else {
        // Deleted since the menu was built.
        refresh_tray_menu();
        return;
    };
    snip::reclip_png(&bytes, &path);
    let Some((bgra, w, h)) = snip::decode_png_bgra(&bytes) else { return };
    let Some(hbmp) = capture::bitmap_from_bgra(&bgra, w, h) else { return };
    let mut pt = windows::Win32::Foundation::POINT::default();
    let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt) };
    thumb::show(hbmp, w, h, path, pt);
}

fn build_tray_menu(app: &AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    let delayed = SubmenuBuilder::new(app, "Delayed snip")
        .item(&MenuItemBuilder::with_id("delay:3", "In 3 seconds").build(app)?)
        .item(&MenuItemBuilder::with_id("delay:5", "In 5 seconds").build(app)?)
        .item(&MenuItemBuilder::with_id("delay:10", "In 10 seconds").build(app)?)
        .build()?;
    let mut recent = SubmenuBuilder::new(app, "Recent snips");
    let files = history::recent(5);
    if files.is_empty() {
        recent = recent.item(
            &MenuItemBuilder::with_id("recent-none", "No snips yet").enabled(false).build(app)?,
        );
    }
    for p in files {
        let id = format!("recent:{}", p.to_string_lossy());
        recent = recent.item(&MenuItemBuilder::with_id(id, history::label(&p)).build(app)?);
    }
    MenuBuilder::new(app)
        .item(&MenuItemBuilder::with_id("snip", "Snip now").build(app)?)
        .item(&MenuItemBuilder::with_id("snip-full", "Snip full screen").build(app)?)
        .item(&delayed)
        .item(&recent.build()?)
        .separator()
        .item(&MenuItemBuilder::with_id("help", "Help").build(app)?)
        .item(&MenuItemBuilder::with_id("settings", "Settings…").build(app)?)
        .item(&MenuItemBuilder::with_id("folder", "Open snips folder").build(app)?)
        .separator()
        .item(&MenuItemBuilder::with_id("quit", "Exit").build(app)?)
        .build()
}

/// Rebuild the tray menu (its Recent snips list changes after each snip).
pub fn refresh_tray_menu() {
    let Some(app) = APP.get() else { return };
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        if let (Ok(menu), Some(tray)) = (build_tray_menu(&app), app.tray_by_id("main")) {
            let _ = tray.set_menu(Some(menu));
        }
    });
}

/// Open the annotation editor for a saved snip (called from the thumbnail).
pub fn open_editor(path: String) {
    let Some(app) = APP.get() else { return };
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        let n = EDITOR_SEQ.fetch_add(1, Ordering::SeqCst);
        let label = format!("editor-{n}");
        EDITOR_PATHS.lock().unwrap().insert(label.clone(), path);
        let _ = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("editor.html".into()))
            .title("SnipTray — Edit snip")
            .inner_size(1100.0, 760.0)
            .min_inner_size(640.0, 480.0)
            .build();
    });
}

fn focused_editor(app: &AppHandle) -> Option<String> {
    app.webview_windows()
        .into_iter()
        .find(|(label, w)| label.starts_with("editor-") && w.is_focused().unwrap_or(false))
        .map(|(label, _)| label)
}

/// The primary monitor's usable area (excluding the taskbar) in logical
/// pixels. Read from Win32: Tauri's `Monitor::work_area` comes back empty.
fn primary_work_area(app: &AppHandle) -> Option<(f64, f64)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
    };
    let scale = app.primary_monitor().ok()??.scale_factor();
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        let hmon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        if !GetMonitorInfoW(hmon, &mut mi).as_bool() {
            return None;
        }
    }
    let r = mi.rcWork;
    let (w, h) = ((r.right - r.left) as f64 / scale, (r.bottom - r.top) as f64 / scale);
    (w > 0.0 && h > 0.0).then_some((w, h))
}

fn open_settings(app: &AppHandle) {
    // Opening Settings ends any prior hotkey-capture state.
    CAPTURING_HOTKEY.store(false, Ordering::SeqCst);
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    // Fit the window to the screen's usable area (minus taskbar and title
    // bar): on small or high-scaling screens the full height doesn't fit.
    // The page scrolls, and the window can be resized either way.
    let (mut w, mut h) = (650.0_f64, 470.0_f64);
    if let Some((aw, ah)) = primary_work_area(app) {
        w = w.min(aw - 32.0);
        h = h.min(ah - 80.0);
    }
    let (min_w, min_h) = (420.0_f64, 320.0_f64);
    if let Ok(win) =
        WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html".into()))
            .title("SnipTray Settings")
            .inner_size(w.max(min_w), h.max(min_h))
            .min_inner_size(min_w, min_h)
            .resizable(true)
            .center()
            .build()
    {
        // If the window is closed mid-capture, don't leave snips suspended.
        win.on_window_event(|event| {
            if matches!(
                event,
                tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
            ) {
                CAPTURING_HOTKEY.store(false, Ordering::SeqCst);
            }
        });
    }
}

/// Open the bundled help page in the default browser.
fn open_help(app: &AppHandle) {
    let Ok(path) = app
        .path()
        .resolve("sniptray-help.html", tauri::path::BaseDirectory::Resource)
    else {
        return;
    };
    let w = util::wide(&path.to_string_lossy());
    unsafe {
        windows::Win32::UI::Shell::ShellExecuteW(
            None,
            windows::core::w!("open"),
            windows::core::PCWSTR(w.as_ptr()),
            None,
            None,
            windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        );
    }
}

fn open_snips_dir() {
    let dir = config::snips_dir(&config::CONFIG.lock().unwrap());
    let _ = std::fs::create_dir_all(&dir);
    let w = util::wide(&dir.to_string_lossy());
    unsafe {
        windows::Win32::UI::Shell::ShellExecuteW(
            None,
            windows::core::w!("open"),
            windows::core::PCWSTR(w.as_ptr()),
            None,
            None,
            windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        );
    }
}

fn pretty_hotkey(hk: &str) -> String {
    hk.split('+')
        .map(|t| match t {
            "Control" => "Ctrl",
            "CommandOrControl" => "Ctrl",
            "PrintScreen" => "PrtSc",
            "Super" => "Win",
            t if t.starts_with("Key") => &t[3..],
            t if t.starts_with("Digit") => &t[5..],
            t => t,
        })
        .collect::<Vec<_>>()
        .join("+")
}

fn tray_tooltip() -> String {
    if HOTKEY_ERROR.lock().unwrap().is_some() {
        return "SnipTray — hotkey unavailable, click to snip".to_string();
    }
    format!(
        "SnipTray — {} to snip",
        pretty_hotkey(&config::CONFIG.lock().unwrap().hotkey)
    )
}

/// 32x32 RGBA tray icon: blue rounded square, white selection frame + handle.
fn tray_icon_image() -> tauri::image::Image<'static> {
    const SZ: usize = 32;
    let mut px = vec![0u8; SZ * SZ * 4];
    let radius = 6.0f64;
    let rounded_alpha = |x: f64, y: f64| -> u8 {
        let (min, max) = (0.5, SZ as f64 - 0.5);
        let cx = x.clamp(min + radius, max - radius);
        let cy = y.clamp(min + radius, max - radius);
        let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
        if d <= radius - 0.5 {
            255
        } else if d >= radius + 0.5 {
            0
        } else {
            ((radius + 0.5 - d) * 255.0) as u8
        }
    };
    for y in 0..SZ {
        for x in 0..SZ {
            let i = (y * SZ + x) * 4;
            let a = rounded_alpha(x as f64 + 0.5, y as f64 + 0.5);
            if a == 0 {
                continue;
            }
            // #3B82F6
            let (mut r, mut g, mut b) = (59u8, 130u8, 246u8);
            let on_frame = ((8..=23).contains(&x)
                && ((8..=9).contains(&y) || (22..=23).contains(&y)))
                || ((8..=23).contains(&y) && ((8..=9).contains(&x) || (22..=23).contains(&x)));
            let on_handle = (19..=26).contains(&x) && (19..=26).contains(&y);
            if on_frame || on_handle {
                r = 255;
                g = 255;
                b = 255;
            }
            px[i] = r;
            px[i + 1] = g;
            px[i + 2] = b;
            px[i + 3] = a;
        }
    }
    tauri::image::Image::new_owned(px, SZ as u32, SZ as u32)
}

// ---------------------------------------------------------------- commands

#[tauri::command]
fn get_settings() -> config::Config {
    config::CONFIG.lock().unwrap().clone()
}

#[tauri::command]
fn get_hotkey_error() -> Option<String> {
    HOTKEY_ERROR.lock().unwrap().clone()
}

#[tauri::command]
fn effective_save_dir() -> String {
    config::snips_dir(&config::CONFIG.lock().unwrap())
        .to_string_lossy()
        .into_owned()
}

#[tauri::command]
fn set_clipboard_path_text(value: bool) {
    let mut c = config::CONFIG.lock().unwrap();
    c.clipboard_path_text = value;
    c.save();
}

#[tauri::command]
fn set_terminal_drop_assist(value: bool) {
    let mut c = config::CONFIG.lock().unwrap();
    c.terminal_drop_assist = value;
    c.save();
}

#[tauri::command]
fn set_thumb_on_snip_monitor(value: bool) {
    let mut c = config::CONFIG.lock().unwrap();
    c.thumb_on_snip_monitor = value;
    c.save();
}

#[tauri::command]
fn set_delete_after_days(value: u32) {
    {
        let mut c = config::CONFIG.lock().unwrap();
        c.delete_after_days = value;
        c.save();
    }
    history::cleanup_async();
}

#[tauri::command]
fn set_save_dir(dir: String) -> String {
    {
        let mut c = config::CONFIG.lock().unwrap();
        c.save_dir = dir;
        c.save();
    }
    // The Recent list follows the snips folder.
    refresh_tray_menu();
    effective_save_dir()
}

#[tauri::command]
fn set_hotkey(app: AppHandle, shortcut: String) -> Result<String, String> {
    let old = config::CONFIG.lock().unwrap().hotkey.clone();
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    match gs.register(shortcut.as_str()) {
        Ok(()) => {
            {
                let mut c = config::CONFIG.lock().unwrap();
                c.hotkey = shortcut.clone();
                c.save();
            }
            // The capture is done; re-enable the handler for the new hotkey.
            CAPTURING_HOTKEY.store(false, Ordering::SeqCst);
            *HOTKEY_ERROR.lock().unwrap() = None;
            if let Some(tray) = app.tray_by_id("main") {
                let _ = tray.set_tooltip(Some(tray_tooltip()));
            }
            Ok(pretty_hotkey(&shortcut))
        }
        Err(e) => {
            let _ = gs.register(old.as_str());
            Err(format!("Couldn't register that combination ({e})"))
        }
    }
}

#[tauri::command]
fn begin_hotkey_capture() {
    // Suspend snips while the user presses keys to pick a new hotkey, so the
    // current combo doesn't trigger a capture mid-reassignment.
    CAPTURING_HOTKEY.store(true, Ordering::SeqCst);
}

#[tauri::command]
fn end_hotkey_capture() {
    // Capture cancelled: the configured hotkey is still registered, so just
    // re-enable the handler.
    CAPTURING_HOTKEY.store(false, Ordering::SeqCst);
}

#[tauri::command]
fn snip_now() {
    snip_on_main_thread();
}

#[tauri::command]
fn open_folder() {
    open_snips_dir();
}

#[tauri::command]
fn get_editor_path(label: String) -> Option<String> {
    EDITOR_PATHS.lock().unwrap().get(&label).cloned()
}

#[tauri::command]
fn read_image(path: String) -> Result<String, String> {
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

#[tauri::command]
fn save_edited(path: String, data_url: String) -> Result<(), String> {
    let b64 = data_url
        .strip_prefix("data:image/png;base64,")
        .ok_or("unexpected data url format")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| e.to_string())?;
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    snip::reclip_png(&bytes, &path);
    Ok(())
}

// --------------------------------------------------------------------- main

fn main() {
    // The thumbnail's drag-out (SHDoDragDrop) requires OLE on the main
    // thread. Tauri/tao only get OLE going once a webview exists, so a
    // drag before any window has opened would fail with
    // CO_E_NOTINITIALIZED. Initialize it up front.
    unsafe {
        let _ = windows::Win32::System::Ole::OleInitialize(None);
    }

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            open_settings(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if CAPTURING_HOTKEY.load(Ordering::SeqCst) {
                        return;
                    }
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    // The global hotkey swallows the keystroke, so a combo
                    // like Ctrl+Shift+Z would break the editor's redo. When
                    // an editor is focused, hand the keys to it instead.
                    if let Some(label) = focused_editor(app) {
                        let hk = config::CONFIG.lock().unwrap().hotkey.clone();
                        let _ = app.emit_to(label.as_str(), "hotkey-passthrough", hk);
                        return;
                    }
                    snip_on_main_thread();
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            get_settings,
            get_hotkey_error,
            effective_save_dir,
            set_clipboard_path_text,
            set_terminal_drop_assist,
            set_thumb_on_snip_monitor,
            set_delete_after_days,
            set_save_dir,
            set_hotkey,
            begin_hotkey_capture,
            end_hotkey_capture,
            snip_now,
            open_folder,
            get_editor_path,
            read_image,
            save_edited
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let _ = APP.set(handle.clone());

            let menu = build_tray_menu(&handle)?;

            TrayIconBuilder::with_id("main")
                .icon(tray_icon_image())
                .tooltip(tray_tooltip())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "snip" => snip_on_main_thread(),
                    "snip-full" => full_snip_on_main_thread(),
                    "help" => open_help(app),
                    "settings" => open_settings(app),
                    "folder" => open_snips_dir(),
                    "quit" => app.exit(0),
                    id => {
                        if let Some(secs) = id.strip_prefix("delay:") {
                            snip_after(secs.parse().unwrap_or(3));
                        } else if let Some(path) = id.strip_prefix("recent:") {
                            open_recent(path.to_string());
                        }
                    }
                })
                .on_tray_icon_event(|_tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        snip_on_main_thread();
                    }
                })
                .build(app)?;

            let hk = config::CONFIG.lock().unwrap().hotkey.clone();
            // Failure is non-fatal, but say so: flag it in the tray tooltip
            // and open Settings, which shows the error, so the user can pick
            // another combo instead of wondering why nothing happens.
            if let Err(e) = handle.global_shortcut().register(hk.as_str()) {
                *HOTKEY_ERROR.lock().unwrap() = Some(format!(
                    "{} couldn't be registered — another app may be using it ({e}). \
                     Pick a different hotkey.",
                    pretty_hotkey(&hk)
                ));
                if let Some(tray) = handle.tray_by_id("main") {
                    let _ = tray.set_tooltip(Some(tray_tooltip()));
                }
                open_settings(&handle);
            }

            // Start with Windows is on by default: enable it once, on first
            // launch, so a later opt-out in Settings sticks.
            {
                let mut c = config::CONFIG.lock().unwrap();
                if c.first_run {
                    let _ = handle.autolaunch().enable();
                    c.first_run = false;
                    c.save();
                }
            }

            // Prune old snips at startup too (the app may run for days, but
            // also may not have run since they aged out).
            history::cleanup_async();
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build tauri app");

    app.run(|_app, event| {
        // Keep running in the tray when the last window closes; only an
        // explicit app.exit() (tray menu "Exit") carries an exit code.
        if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
            if code.is_none() {
                api.prevent_exit();
            }
        }
    });
}
