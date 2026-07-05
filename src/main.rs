#![windows_subsystem = "windows"]
#![allow(clippy::missing_safety_doc)]

mod capture;
mod config;
mod hotkey_ui;
mod overlay;
mod settings;
mod snip;
mod thumb;
mod tray;
mod util;

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::OleInitialize;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const WM_TRAYICON: u32 = WM_APP + 1;
pub const HOTKEY_ID: i32 = 1;

pub const CMD_SNIP: usize = 101;
pub const CMD_FOLDER: usize = 104;
pub const CMD_EXIT: usize = 105;
pub const CMD_SETTINGS: usize = 106;

thread_local! {
    pub static CONFIG: RefCell<config::Config> = RefCell::new(config::Config::default());
}

pub static OVERLAY_ACTIVE: AtomicBool = AtomicBool::new(false);
static TASKBAR_CREATED_MSG: OnceLock<u32> = OnceLock::new();

fn main() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        // Hidden self-test: `sniptray --test-capture out.png` captures the
        // whole virtual screen to a PNG and exits.
        let args: Vec<String> = std::env::args().collect();
        if let Some(i) = args.iter().position(|a| a == "--test-capture") {
            let out = args
                .get(i + 1)
                .cloned()
                .unwrap_or_else(|| "capture_test.png".to_string());
            let code = if test_capture(&out) { 0 } else { 1 };
            std::process::exit(code);
        }

        // Single instance.
        let _mutex = CreateMutexW(None, false, w!("SnipTray_single_instance"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }

        let _ = OleInitialize(None);

        CONFIG.with(|c| *c.borrow_mut() = config::Config::load());

        let hinstance: HINSTANCE = GetModuleHandleW(None).unwrap().into();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(main_wndproc),
            hInstance: hinstance,
            lpszClassName: w!("SnipTrayMain"),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("SnipTrayMain"),
            w!("SnipTray"),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance),
            None,
        )
        .expect("failed to create main window");

        let _ = TASKBAR_CREATED_MSG.set(RegisterWindowMessageW(w!("TaskbarCreated")));

        tray::add(hwnd);
        tray::update_tooltip(hwnd);

        if !register_hotkey(hwnd) {
            tray::balloon(
                hwnd,
                "Hotkey unavailable",
                "Couldn't register the snip hotkey — another app owns it. Right-click the tray icon → Set hotkey.",
            );
        } else if CONFIG.with(|c| c.borrow().first_run) {
            let hk = CONFIG.with(|c| c.borrow().hotkey_string());
            tray::balloon(
                hwnd,
                "SnipTray is running",
                &format!("Press {hk} to snip. Drag the preview into any app — the image is also on your clipboard."),
            );
            CONFIG.with(|c| {
                let mut cfg = c.borrow_mut();
                cfg.first_run = false;
                cfg.save();
            });
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// (Re-)register the global hotkey from the current config.
pub fn register_hotkey(hwnd: HWND) -> bool {
    unsafe {
        let _ = UnregisterHotKey(Some(hwnd), HOTKEY_ID);
        let (mods, vk) = CONFIG.with(|c| {
            let cfg = c.borrow();
            (cfg.hotkey_mods, cfg.hotkey_vk)
        });
        RegisterHotKey(
            Some(hwnd),
            HOTKEY_ID,
            HOT_KEY_MODIFIERS(mods | MOD_NOREPEAT.0),
            vk,
        )
        .is_ok()
    }
}

fn start_snip(main_hwnd: HWND) {
    if OVERLAY_ACTIVE.swap(true, Ordering::SeqCst) {
        return;
    }
    let ok = capture::capture_virtual_screen()
        .map(|shot| overlay::show(shot, main_hwnd).is_ok())
        .unwrap_or(false);
    if !ok {
        OVERLAY_ACTIVE.store(false, Ordering::SeqCst);
    }
}

fn open_snips_folder() {
    let dir = CONFIG.with(|c| config::snips_dir(&c.borrow()));
    let _ = std::fs::create_dir_all(&dir);
    let w = util::wide(&dir.to_string_lossy());
    unsafe {
        ShellExecuteW(None, w!("open"), PCWSTR(w.as_ptr()), None, None, SW_SHOWNORMAL);
    }
}

unsafe extern "system" fn main_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if Some(&msg) == TASKBAR_CREATED_MSG.get() {
        // Explorer restarted: put the tray icon back.
        tray::add(hwnd);
        tray::update_tooltip(hwnd);
        return LRESULT(0);
    }
    match msg {
        WM_HOTKEY => {
            if wparam.0 as i32 == HOTKEY_ID {
                start_snip(hwnd);
            }
            LRESULT(0)
        }
        WM_TRAYICON => {
            match lparam.0 as u32 {
                WM_LBUTTONUP => start_snip(hwnd),
                WM_RBUTTONUP | WM_CONTEXTMENU => tray::show_menu(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            match util::loword_w(wparam) as usize {
                CMD_SNIP => start_snip(hwnd),
                CMD_SETTINGS => settings::open(hwnd),
                CMD_FOLDER => open_snips_folder(),
                CMD_EXIT => {
                    let _ = DestroyWindow(hwnd);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            tray::remove(hwnd);
            let _ = UnregisterHotKey(Some(hwnd), HOTKEY_ID);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn test_capture(out: &str) -> bool {
    let Some(shot) = capture::capture_virtual_screen() else {
        return false;
    };
    let Some(bgra) = capture::bgra_pixels(shot.hbmp, shot.w, shot.h) else {
        return false;
    };
    let Some(png) = snip::encode_png(&bgra, shot.w, shot.h) else {
        return false;
    };
    std::fs::write(out, png).is_ok()
}
