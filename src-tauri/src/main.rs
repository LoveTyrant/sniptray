#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(clippy::missing_safety_doc)]

mod capture;
mod config;
mod overlay;
mod snip;
mod thumb;
mod util;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{LazyLock, Mutex, OnceLock};

use base64::Engine;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

pub static OVERLAY_ACTIVE: AtomicBool = AtomicBool::new(false);
static APP: OnceLock<AppHandle> = OnceLock::new();
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

fn open_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let _ = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html".into()))
        .title("SnipTray Settings")
        .inner_size(500.0, 570.0)
        .resizable(false)
        .maximizable(false)
        .build();
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
fn set_save_dir(dir: String) -> String {
    {
        let mut c = config::CONFIG.lock().unwrap();
        c.save_dir = dir;
        c.save();
    }
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
                .with_handler(|_app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        snip_on_main_thread();
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            get_settings,
            effective_save_dir,
            set_clipboard_path_text,
            set_terminal_drop_assist,
            set_save_dir,
            set_hotkey,
            snip_now,
            open_folder,
            get_editor_path,
            read_image,
            save_edited
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let _ = APP.set(handle.clone());

            let snip_i = MenuItemBuilder::with_id("snip", "Snip now").build(app)?;
            let settings_i = MenuItemBuilder::with_id("settings", "Settings…").build(app)?;
            let folder_i = MenuItemBuilder::with_id("folder", "Open snips folder").build(app)?;
            let quit_i = MenuItemBuilder::with_id("quit", "Exit").build(app)?;
            let menu = MenuBuilder::new(app)
                .items(&[&snip_i, &settings_i, &folder_i])
                .separator()
                .item(&quit_i)
                .build()?;

            TrayIconBuilder::with_id("main")
                .icon(tray_icon_image())
                .tooltip(tray_tooltip())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "snip" => snip_on_main_thread(),
                    "settings" => open_settings(app),
                    "folder" => open_snips_dir(),
                    "quit" => app.exit(0),
                    _ => {}
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
            // Failure is non-fatal: the user can pick another combo in Settings.
            let _ = handle.global_shortcut().register(hk.as_str());
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
