use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{
    FileOpenDialog, IFileOpenDialog, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::util::wide;
use crate::{config, hotkey_ui};

static CLASS_REGISTERED: AtomicBool = AtomicBool::new(false);
const CLASS_NAME: PCWSTR = w!("SnipTraySettings");

const ID_HOTKEY_VALUE: i32 = 201;
const ID_HOTKEY_CHANGE: i32 = 202;
const ID_CHK_STARTUP: i32 = 203;
const ID_CHK_PATHTEXT: i32 = 204;
const ID_EDIT_DIR: i32 = 205;
const ID_BTN_BROWSE: i32 = 206;
const ID_BTN_DEFAULT: i32 = 207;
const ID_BTN_CLOSE: i32 = 208;

thread_local! {
    static OPEN_HWND: Cell<isize> = const { Cell::new(0) };
}

struct SettingsState {
    main_hwnd: HWND,
    font: HFONT,
    font_bold: HFONT,
}

pub fn open(main_hwnd: HWND) {
    unsafe {
        let existing = OPEN_HWND.with(|c| c.get());
        if existing != 0 {
            let _ = SetForegroundWindow(HWND(existing as *mut _));
            return;
        }
        let Ok(hmodule) = GetModuleHandleW(None) else { return };
        let hinstance: HINSTANCE = hmodule.into();
        if !CLASS_REGISTERED.swap(true, Ordering::SeqCst) {
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinstance,
                lpszClassName: CLASS_NAME,
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hbrBackground: HBRUSH(((COLOR_BTNFACE.0 + 1) as usize) as *mut _),
                ..Default::default()
            };
            RegisterClassW(&wc);
        }

        let state = Box::new(SettingsState {
            main_hwnd,
            font: HFONT::default(),
            font_bold: HFONT::default(),
        });

        let sw = GetSystemMetrics(SM_CXSCREEN);
        let sh = GetSystemMetrics(SM_CYSCREEN);
        let (w, h) = (470, 268);
        let style = WS_CAPTION | WS_SYSMENU | WS_POPUP;
        let mut rect = RECT { left: 0, top: 0, right: w, bottom: h };
        let _ = AdjustWindowRectEx(&mut rect, style, false, WS_EX_TOPMOST);
        let ww = rect.right - rect.left;
        let wh = rect.bottom - rect.top;

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST,
            CLASS_NAME,
            w!("SnipTray Settings"),
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
        }
    }
}

/// Called by the hotkey-capture window when the binding may have changed.
pub fn refresh_hotkey_label() {
    let hwnd_val = OPEN_HWND.with(|c| c.get());
    if hwnd_val == 0 {
        return;
    }
    unsafe {
        let hwnd = HWND(hwnd_val as *mut _);
        if let Ok(label) = GetDlgItem(Some(hwnd), ID_HOTKEY_VALUE) {
            let hk = crate::CONFIG.with(|c| c.borrow().hotkey_string());
            let w = wide(&hk);
            let _ = SetWindowTextW(label, PCWSTR(w.as_ptr()));
        }
    }
}

unsafe fn state<'a>(hwnd: HWND) -> Option<&'a mut SettingsState> {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SettingsState;
    ptr.as_mut()
}

unsafe fn mk_child(
    parent: HWND,
    class: PCWSTR,
    text: &str,
    style: WINDOW_STYLE,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: i32,
    font: HFONT,
) -> HWND {
    let tw = wide(text);
    let hinstance: HINSTANCE = GetModuleHandleW(None).unwrap_or_default().into();
    let hwnd = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        class,
        PCWSTR(tw.as_ptr()),
        WS_CHILD | WS_VISIBLE | style,
        x,
        y,
        w,
        h,
        Some(parent),
        Some(HMENU(id as usize as *mut _)),
        Some(hinstance),
        None,
    )
    .unwrap_or_default();
    SendMessageW(hwnd, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
    hwnd
}

unsafe fn set_check(hwnd: HWND, id: i32, checked: bool) {
    if let Ok(chk) = GetDlgItem(Some(hwnd), id) {
        SendMessageW(
            chk,
            BM_SETCHECK,
            Some(WPARAM(if checked { 1 } else { 0 })),
            Some(LPARAM(0)),
        );
    }
}

unsafe fn get_check(hwnd: HWND, id: i32) -> bool {
    GetDlgItem(Some(hwnd), id)
        .map(|chk| SendMessageW(chk, BM_GETCHECK, Some(WPARAM(0)), Some(LPARAM(0))).0 == 1)
        .unwrap_or(false)
}

unsafe fn set_dir_text(hwnd: HWND) {
    if let Ok(edit) = GetDlgItem(Some(hwnd), ID_EDIT_DIR) {
        let dir = crate::CONFIG.with(|c| config::snips_dir(&c.borrow()));
        let w = wide(&dir.to_string_lossy());
        let _ = SetWindowTextW(edit, PCWSTR(w.as_ptr()));
    }
}

unsafe fn browse_folder(hwnd: HWND) -> Option<String> {
    let dlg: IFileOpenDialog =
        CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
    let opts = dlg.GetOptions().ok()?;
    dlg.SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).ok()?;
    dlg.Show(Some(hwnd)).ok()?; // Err = user cancelled
    let item = dlg.GetResult().ok()?;
    let pw = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
    let s = pw.to_string().ok();
    CoTaskMemFree(Some(pw.0 as _));
    s
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
                let s = |v: i32| v * dpi / 96;
                let mk_font = |px: i32, weight: i32| {
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
                st.font = mk_font(12, FW_NORMAL.0 as i32);
                st.font_bold = mk_font(12, FW_SEMIBOLD.0 as i32);

                let stc = w!("STATIC");
                let btn = w!("BUTTON");
                let edt = w!("EDIT");
                let hk = crate::CONFIG.with(|c| c.borrow().hotkey_string());

                mk_child(hwnd, stc, "Snip hotkey:", WINDOW_STYLE(0), s(20), s(24), s(90), s(20), 0, st.font);
                mk_child(hwnd, stc, &hk, WINDOW_STYLE(0), s(115), s(24), s(200), s(20), ID_HOTKEY_VALUE, st.font_bold);
                mk_child(hwnd, btn, "Change…", WINDOW_STYLE(BS_PUSHBUTTON as u32), s(340), s(18), s(105), s(30), ID_HOTKEY_CHANGE, st.font);

                mk_child(hwnd, btn, "Start with Windows", WINDOW_STYLE(BS_AUTOCHECKBOX as u32), s(20), s(62), s(420), s(24), ID_CHK_STARTUP, st.font);
                mk_child(
                    hwnd,
                    btn,
                    "Also copy file path as text — lets terminal apps (Claude Code) paste the snip",
                    WINDOW_STYLE((BS_AUTOCHECKBOX | BS_MULTILINE) as u32),
                    s(20),
                    s(92),
                    s(425),
                    s(36),
                    ID_CHK_PATHTEXT,
                    st.font,
                );

                mk_child(hwnd, stc, "Save snips to:", WINDOW_STYLE(0), s(20), s(140), s(200), s(20), 0, st.font);
                mk_child(hwnd, edt, "", WINDOW_STYLE((ES_READONLY | ES_AUTOHSCROLL) as u32) | WS_BORDER, s(20), s(162), s(265), s(24), ID_EDIT_DIR, st.font);
                mk_child(hwnd, btn, "Browse…", WINDOW_STYLE(BS_PUSHBUTTON as u32), s(293), s(160), s(80), s(27), ID_BTN_BROWSE, st.font);
                mk_child(hwnd, btn, "Default", WINDOW_STYLE(BS_PUSHBUTTON as u32), s(379), s(160), s(66), s(27), ID_BTN_DEFAULT, st.font);

                mk_child(hwnd, btn, "Close", WINDOW_STYLE(BS_DEFPUSHBUTTON as u32), s(345), s(212), s(100), s(32), ID_BTN_CLOSE, st.font);

                set_check(hwnd, ID_CHK_STARTUP, config::startup_enabled());
                set_check(
                    hwnd,
                    ID_CHK_PATHTEXT,
                    crate::CONFIG.with(|c| c.borrow().clipboard_path_text),
                );
                set_dir_text(hwnd);
            }
            LRESULT(0)
        }
        WM_CTLCOLORSTATIC => {
            let hdc = HDC(wparam.0 as *mut _);
            SetBkMode(hdc, TRANSPARENT);
            LRESULT(GetSysColorBrush(COLOR_BTNFACE).0 as isize)
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xffff) as i32;
            match id {
                ID_HOTKEY_CHANGE => {
                    if let Some(st) = state(hwnd) {
                        hotkey_ui::open(st.main_hwnd);
                    }
                }
                ID_CHK_STARTUP => {
                    config::set_startup(get_check(hwnd, ID_CHK_STARTUP));
                }
                ID_CHK_PATHTEXT => {
                    let checked = get_check(hwnd, ID_CHK_PATHTEXT);
                    crate::CONFIG.with(|c| {
                        let mut cfg = c.borrow_mut();
                        cfg.clipboard_path_text = checked;
                        cfg.save();
                    });
                }
                ID_BTN_BROWSE => {
                    if let Some(dir) = browse_folder(hwnd) {
                        crate::CONFIG.with(|c| {
                            let mut cfg = c.borrow_mut();
                            cfg.save_dir = dir;
                            cfg.save();
                        });
                        set_dir_text(hwnd);
                    }
                }
                ID_BTN_DEFAULT => {
                    crate::CONFIG.with(|c| {
                        let mut cfg = c.borrow_mut();
                        cfg.save_dir = String::new();
                        cfg.save();
                    });
                    set_dir_text(hwnd);
                }
                ID_BTN_CLOSE => {
                    let _ = DestroyWindow(hwnd);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_NCDESTROY => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SettingsState;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            OPEN_HWND.with(|c| c.set(0));
            if !ptr.is_null() {
                let st = Box::from_raw(ptr);
                if !st.font.is_invalid() {
                    let _ = DeleteObject(st.font.into());
                }
                if !st.font_bold.is_invalid() {
                    let _ = DeleteObject(st.font_bold.into());
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
