use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::util::wide;
use crate::{config, HOTKEY_ID};

static CLASS_REGISTERED: AtomicBool = AtomicBool::new(false);
const CLASS_NAME: PCWSTR = w!("SnipTrayHotkey");

thread_local! {
    static OPEN_HWND: Cell<isize> = const { Cell::new(0) };
}

struct HkState {
    main_hwnd: HWND,
    preview: String,
    status: String,
    font_small: HFONT,
    font_big: HFONT,
}

pub fn open(main_hwnd: HWND) {
    unsafe {
        let existing = OPEN_HWND.with(|c| c.get());
        if existing != 0 {
            let _ = SetForegroundWindow(HWND(existing as *mut _));
            return;
        }

        // Release the hotkey while capturing so the user can press the current combo too.
        let _ = UnregisterHotKey(Some(main_hwnd), HOTKEY_ID);

        let Ok(hmodule) = GetModuleHandleW(None) else { return };
        let hinstance: HINSTANCE = hmodule.into();
        if !CLASS_REGISTERED.swap(true, Ordering::SeqCst) {
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinstance,
                lpszClassName: CLASS_NAME,
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hbrBackground: HBRUSH(((COLOR_WINDOW.0 + 1) as usize) as *mut _),
                ..Default::default()
            };
            RegisterClassW(&wc);
        }

        let cur = crate::CONFIG.with(|c| c.borrow().hotkey_string());
        let state = Box::new(HkState {
            main_hwnd,
            preview: cur,
            status: String::new(),
            font_small: HFONT::default(),
            font_big: HFONT::default(),
        });

        let sw = GetSystemMetrics(SM_CXSCREEN);
        let sh = GetSystemMetrics(SM_CYSCREEN);
        // Rough size; per-monitor DPI handled well enough for a transient dialog.
        let (w, h) = (440, 200);
        let style = WS_CAPTION | WS_SYSMENU | WS_POPUP;
        let mut rect = RECT { left: 0, top: 0, right: w, bottom: h };
        let _ = AdjustWindowRectEx(&mut rect, style, false, WS_EX_TOPMOST);
        let ww = rect.right - rect.left;
        let wh = rect.bottom - rect.top;

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST,
            CLASS_NAME,
            w!("SnipTray — set hotkey"),
            style,
            (sw - ww) / 2,
            (sh - wh) / 2,
            ww,
            wh,
            None,
            None,
            Some(hinstance),
            Some(Box::into_raw(state) as *const _),
        );
        if let Ok(hwnd) = hwnd {
            OPEN_HWND.with(|c| c.set(hwnd.0 as isize));
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        } else {
            // Window creation failed; restore the hotkey.
            crate::register_hotkey(main_hwnd);
        }
    }
}

unsafe fn state<'a>(hwnd: HWND) -> Option<&'a mut HkState> {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut HkState;
    ptr.as_mut()
}

fn mods_now() -> u32 {
    unsafe {
        let mut m = 0;
        if GetKeyState(VK_CONTROL.0 as i32) < 0 {
            m |= MOD_CONTROL.0;
        }
        if GetKeyState(VK_MENU.0 as i32) < 0 {
            m |= MOD_ALT.0;
        }
        if GetKeyState(VK_SHIFT.0 as i32) < 0 {
            m |= MOD_SHIFT.0;
        }
        if GetKeyState(VK_LWIN.0 as i32) < 0 || GetKeyState(VK_RWIN.0 as i32) < 0 {
            m |= MOD_WIN.0;
        }
        m
    }
}

fn mods_prefix(m: u32) -> String {
    let mut s = String::new();
    if m & MOD_CONTROL.0 != 0 {
        s.push_str("Ctrl+");
    }
    if m & MOD_ALT.0 != 0 {
        s.push_str("Alt+");
    }
    if m & MOD_SHIFT.0 != 0 {
        s.push_str("Shift+");
    }
    if m & MOD_WIN.0 != 0 {
        s.push_str("Win+");
    }
    s
}

fn standalone_ok(vk: u16) -> bool {
    (vk >= VK_F1.0 && vk <= VK_F24.0)
        || vk == VK_SNAPSHOT.0
        || vk == VK_PAUSE.0
        || vk == VK_SCROLL.0
}

fn is_modifier(vk: u16) -> bool {
    matches!(
        VIRTUAL_KEY(vk),
        VK_SHIFT | VK_CONTROL | VK_MENU | VK_LWIN | VK_RWIN | VK_LSHIFT | VK_RSHIFT
            | VK_LCONTROL | VK_RCONTROL | VK_LMENU | VK_RMENU
    )
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCCREATE => {
            let cs = lparam.0 as *const CREATESTRUCTW;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*cs).lpCreateParams as isize);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CREATE => {
            if let Some(st) = state(hwnd) {
                let dpi = GetDpiForWindow(hwnd) as i32;
                let mk = |px: i32, weight: i32| {
                    CreateFontW(
                        -(px * dpi) / 96,
                        0,
                        0,
                        0,
                        weight,
                        0,
                        0,
                        0,
                        DEFAULT_CHARSET,
                        OUT_DEFAULT_PRECIS,
                        CLIP_DEFAULT_PRECIS,
                        CLEARTYPE_QUALITY,
                        DEFAULT_PITCH.0 as u32,
                        w!("Segoe UI"),
                    )
                };
                st.font_small = mk(12, FW_NORMAL.0 as i32);
                st.font_big = mk(20, FW_SEMIBOLD.0 as i32);
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            if let Some(st) = state(hwnd) {
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                let dpi = GetDpiForWindow(hwnd) as i32;
                let s = |v: i32| v * dpi / 96;
                SetBkMode(hdc, TRANSPARENT);

                let of = SelectObject(hdc, st.font_small.into());
                SetTextColor(hdc, COLORREF(0x00303030));
                let mut t1 = wide("Press the new key combination for snipping:");
                let mut r1 = RECT { left: s(20), top: s(18), right: rc.right - s(20), bottom: s(40) };
                let _ = DrawTextW(hdc, &mut t1, &mut r1, DT_SINGLELINE | DT_LEFT);

                SelectObject(hdc, st.font_big.into());
                SetTextColor(hdc, COLORREF(0x00EB6325)); // blue-ish (BGR)
                let mut t2 = wide(&st.preview);
                let mut r2 = RECT { left: s(20), top: s(52), right: rc.right - s(20), bottom: s(95) };
                let _ = DrawTextW(hdc, &mut t2, &mut r2, DT_SINGLELINE | DT_CENTER);

                SelectObject(hdc, st.font_small.into());
                SetTextColor(hdc, COLORREF(0x002828C8)); // red (BGR)
                let mut t3 = wide(&st.status);
                let mut r3 = RECT { left: s(20), top: s(108), right: rc.right - s(20), bottom: s(130) };
                let _ = DrawTextW(hdc, &mut t3, &mut r3, DT_SINGLELINE | DT_CENTER);

                SetTextColor(hdc, COLORREF(0x00808080));
                let mut t4 = wide("Plain letters need Ctrl / Alt / Shift / Win.");
                let mut r4 = RECT { left: s(20), top: s(142), right: rc.right - s(20), bottom: s(162) };
                let _ = DrawTextW(hdc, &mut t4, &mut r4, DT_SINGLELINE | DT_LEFT);
                let mut t5 = wide("F-keys and PrtSc also work alone.  Esc cancels.");
                let mut r5 = RECT { left: s(20), top: s(162), right: rc.right - s(20), bottom: s(182) };
                let _ = DrawTextW(hdc, &mut t5, &mut r5, DT_SINGLELINE | DT_LEFT);
                SelectObject(hdc, of);
            }
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let vk = wparam.0 as u16;
            if let Some(st) = state(hwnd) {
                if vk == VK_ESCAPE.0 {
                    let _ = DestroyWindow(hwnd);
                    return LRESULT(0);
                }
                if is_modifier(vk) {
                    st.preview = format!("{}…", mods_prefix(mods_now()));
                    st.status.clear();
                    let _ = InvalidateRect(Some(hwnd), None, true);
                    return LRESULT(0);
                }
                let mods = mods_now();
                if mods == 0 && !standalone_ok(vk) {
                    st.preview = config::vk_name(vk as u32);
                    st.status =
                        "Add Ctrl / Alt / Shift / Win, or use an F-key or PrtSc.".to_string();
                    let _ = InvalidateRect(Some(hwnd), None, true);
                    return LRESULT(0);
                }
                // Try to claim it system-wide.
                if RegisterHotKey(
                    Some(st.main_hwnd),
                    HOTKEY_ID,
                    HOT_KEY_MODIFIERS(mods | MOD_NOREPEAT.0),
                    vk as u32,
                )
                .is_ok()
                {
                    crate::CONFIG.with(|c| {
                        let mut cfg = c.borrow_mut();
                        cfg.hotkey_mods = mods;
                        cfg.hotkey_vk = vk as u32;
                        cfg.save();
                    });
                    let _ = DestroyWindow(hwnd);
                } else {
                    st.preview = config::hotkey_string(mods, vk as u32);
                    st.status = "That combination is already used by another app.".to_string();
                    let _ = InvalidateRect(Some(hwnd), None, true);
                }
            }
            LRESULT(0)
        }
        WM_KEYUP | WM_SYSKEYUP => {
            let vk = wparam.0 as u16;
            if is_modifier(vk) {
                if let Some(st) = state(hwnd) {
                    let m = mods_now();
                    st.preview = if m == 0 {
                        crate::CONFIG.with(|c| c.borrow().hotkey_string())
                    } else {
                        format!("{}…", mods_prefix(m))
                    };
                    let _ = InvalidateRect(Some(hwnd), None, true);
                }
            }
            LRESULT(0)
        }
        WM_NCDESTROY => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut HkState;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            OPEN_HWND.with(|c| c.set(0));
            if !ptr.is_null() {
                let st = Box::from_raw(ptr);
                if !st.font_small.is_invalid() {
                    let _ = DeleteObject(st.font_small.into());
                }
                if !st.font_big.is_invalid() {
                    let _ = DeleteObject(st.font_big.into());
                }
                // Re-register whatever the config now holds (new combo, or the
                // old one if the dialog was cancelled).
                let _ = UnregisterHotKey(Some(st.main_hwnd), HOTKEY_ID);
                crate::register_hotkey(st.main_hwnd);
                crate::tray::update_tooltip(st.main_hwnd);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
