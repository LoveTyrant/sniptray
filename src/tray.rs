use std::ffi::c_void;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::util::copy_wstr;
use crate::{CMD_EXIT, CMD_FOLDER, CMD_SETTINGS, CMD_SNIP, WM_TRAYICON};

const TRAY_ID: u32 = 1;

fn base_nid(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        ..Default::default()
    }
}

pub fn add(hwnd: HWND) {
    unsafe {
        let mut nid = base_nid(hwnd);
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = WM_TRAYICON;
        nid.hIcon = make_icon();
        copy_wstr(&mut nid.szTip, "SnipTray");
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
    }
}

pub fn update_tooltip(hwnd: HWND) {
    unsafe {
        let hk = crate::CONFIG.with(|c| c.borrow().hotkey_string());
        let mut nid = base_nid(hwnd);
        nid.uFlags = NIF_TIP;
        copy_wstr(&mut nid.szTip, &format!("SnipTray — {hk} to snip"));
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

pub fn balloon(hwnd: HWND, title: &str, text: &str) {
    unsafe {
        let mut nid = base_nid(hwnd);
        nid.uFlags = NIF_INFO;
        nid.dwInfoFlags = NIIF_INFO;
        copy_wstr(&mut nid.szInfoTitle, title);
        copy_wstr(&mut nid.szInfo, text);
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

pub fn remove(hwnd: HWND) {
    unsafe {
        let nid = base_nid(hwnd);
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

pub fn show_menu(hwnd: HWND) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let hk = crate::CONFIG.with(|c| c.borrow().hotkey_string());
        append(menu, CMD_SNIP, &format!("Snip now\t{hk}"), false);
        append(menu, CMD_SETTINGS, "Settings…", false);
        append(menu, CMD_FOLDER, "Open snips folder", false);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        append(menu, CMD_EXIT, "Exit", false);

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON,
            pt.x,
            pt.y,
            Some(0),
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
    }
}

unsafe fn append(menu: HMENU, id: usize, text: &str, checked: bool) {
    let w = crate::util::wide(text);
    let mut flags = MF_STRING;
    if checked {
        flags |= MF_CHECKED;
    }
    let _ = AppendMenuW(menu, flags, id, PCWSTR(w.as_ptr()));
}

/// Simple programmatic tray icon: blue rounded square with a white
/// selection frame and a corner handle.
fn make_icon() -> HICON {
    const SZ: usize = 32;
    let mut px = vec![0u8; SZ * SZ * 4]; // BGRA
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
            // Background #3B82F6 (BGR).
            let (mut b, mut g, mut r) = (246u8, 130u8, 59u8);
            // White selection frame, 2 px stroke, inset 8..=23.
            let on_frame = ((8..=23).contains(&x) && ((8..=9).contains(&y) || (22..=23).contains(&y)))
                || ((8..=23).contains(&y) && ((8..=9).contains(&x) || (22..=23).contains(&x)));
            // Filled handle square at the bottom-right frame corner.
            let on_handle = (19..=26).contains(&x) && (19..=26).contains(&y);
            if on_frame || on_handle {
                b = 255;
                g = 255;
                r = 255;
            }
            px[i] = b;
            px[i + 1] = g;
            px[i + 2] = r;
            px[i + 3] = a;
        }
    }

    unsafe {
        let color = CreateBitmap(
            SZ as i32,
            SZ as i32,
            1,
            32,
            Some(px.as_ptr() as *const c_void),
        );
        let mask_bits = [0u8; SZ * SZ / 8];
        let mask = CreateBitmap(
            SZ as i32,
            SZ as i32,
            1,
            1,
            Some(mask_bits.as_ptr() as *const c_void),
        );
        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info).unwrap_or_default();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}
