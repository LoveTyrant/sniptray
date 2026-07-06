use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::{
    IDataObject, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{
    CF_UNICODETEXT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_NONE, IDropSource,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, VK_ESCAPE};
use windows::Win32::UI::Shell::{
    SHCreateItemFromParsingName, SHDoDragDrop, BHID_DataObject, IShellItem,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::util::{wide, x_lparam, y_lparam};

static CLASS_REGISTERED: AtomicBool = AtomicBool::new(false);
const CLASS_NAME: PCWSTR = w!("SnipTrayThumb");

struct ThumbState {
    hbmp: HBITMAP,
    img_w: i32,
    img_h: i32,
    thumb_w: i32,
    thumb_h: i32,
    path: String,
    font: HFONT,
    press: Option<POINT>,
}

/// Show the floating preview. Takes ownership of `hbmp`.
pub fn show(hbmp: HBITMAP, img_w: i32, img_h: i32, path: String) {
    unsafe {
        let Ok(hmodule) = GetModuleHandleW(None) else {
            let _ = DeleteObject(hbmp.into());
            return;
        };
        let hinstance: HINSTANCE = hmodule.into();
        if !CLASS_REGISTERED.swap(true, Ordering::SeqCst) {
            let wc = WNDCLASSW {
                style: CS_DBLCLKS | CS_DROPSHADOW,
                lpfnWndProc: Some(wndproc),
                hInstance: hinstance,
                lpszClassName: CLASS_NAME,
                hCursor: LoadCursorW(None, IDC_SIZEALL).unwrap_or_default(),
                ..Default::default()
            };
            RegisterClassW(&wc);
        }

        // Scale so the longest edge is at most ~280 px.
        let max_edge = 280.0f64;
        let scale = (max_edge / img_w.max(img_h) as f64).min(1.0);
        let thumb_w = ((img_w as f64 * scale).round() as i32).max(1);
        let thumb_h = ((img_h as f64 * scale).round() as i32).max(1);
        let bar_h = 20;
        let border = 1;
        let win_w = thumb_w + border * 2;
        let win_h = thumb_h + bar_h + border * 2;

        // Pin to the bottom-right of the primary monitor's work area,
        // just above the taskbar clock.
        let hmon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(hmon, &mut mi);
        let wa = mi.rcWork;
        let margin = 12;
        let x = (wa.right - win_w - margin).max(wa.left);
        let y = (wa.bottom - win_h - margin).max(wa.top);

        let state = Box::new(ThumbState {
            hbmp,
            img_w,
            img_h,
            thumb_w,
            thumb_h,
            path,
            font: HFONT::default(),
            press: None,
        });
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            CLASS_NAME,
            w!("SnipTray preview"),
            WS_POPUP,
            x,
            y,
            win_w,
            win_h,
            None,
            None,
            Some(hinstance),
            Some(Box::into_raw(state) as *const _),
        );
        if let Ok(hwnd) = hwnd {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
    }
}

unsafe fn state<'a>(hwnd: HWND) -> Option<&'a mut ThumbState> {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ThumbState;
    ptr.as_mut()
}

unsafe fn do_drag(hwnd: HWND, path: &str) -> bool {
    let hs = HSTRING::from(path);
    let item: IShellItem = match SHCreateItemFromParsingName(&hs, None) {
        Ok(i) => i,
        Err(_) => return false,
    };
    let dobj: IDataObject = match item.BindToHandler(None, &BHID_DataObject) {
        Ok(d) => d,
        Err(_) => return false,
    };
    // Also offer the path as plain text. Some drop targets (e.g. terminals
    // that hijack file drops for their own UI) will insert dropped text into
    // the prompt instead — which is exactly what a TUI app such as Claude
    // Code needs to pick the image up.
    add_text_format(&dobj, path);
    let effect = SHDoDragDrop(
        Some(hwnd),
        &dobj,
        None::<&IDropSource>,
        DROPEFFECT_COPY | DROPEFFECT_LINK,
    )
    .unwrap_or(DROPEFFECT_NONE);
    effect != DROPEFFECT_NONE
}

/// Attach a CF_UNICODETEXT rendition of the file path to the drag data object.
unsafe fn add_text_format(dobj: &IDataObject, path: &str) {
    let w: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = w.len() * 2;
    let Ok(hglobal) = GlobalAlloc(GMEM_MOVEABLE, bytes) else {
        return;
    };
    let ptr = GlobalLock(hglobal);
    if ptr.is_null() {
        return;
    }
    std::ptr::copy_nonoverlapping(w.as_ptr() as *const u8, ptr as *mut u8, bytes);
    let _ = GlobalUnlock(hglobal);
    let fe = FORMATETC {
        cfFormat: CF_UNICODETEXT.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0 as u32,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let sm = STGMEDIUM {
        tymed: TYMED_HGLOBAL.0 as u32,
        u: STGMEDIUM_0 { hGlobal: hglobal },
        pUnkForRelease: std::mem::ManuallyDrop::new(None),
    };
    // fRelease = true: the data object takes ownership of the HGLOBAL.
    let _ = dobj.SetData(&fe, &sm, true);
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
                let px = -(11 * dpi) / 96;
                st.font = CreateFontW(
                    px,
                    0,
                    0,
                    0,
                    FW_NORMAL.0 as i32,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET,
                    OUT_DEFAULT_PRECIS,
                    CLIP_DEFAULT_PRECIS,
                    CLEARTYPE_QUALITY,
                    DEFAULT_PITCH.0 as u32,
                    w!("Segoe UI"),
                );
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            if let Some(st) = state(hwnd) {
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                // Border.
                let border_br = CreateSolidBrush(COLORREF(0x00464646));
                FillRect(hdc, &rc, border_br);
                let _ = DeleteObject(border_br.into());
                // Image.
                let src = CreateCompatibleDC(Some(hdc));
                let old = SelectObject(src, st.hbmp.into());
                SetStretchBltMode(hdc, HALFTONE);
                let mut org = POINT::default();
                let _ = SetBrushOrgEx(hdc, 0, 0, Some(&mut org));
                let _ = StretchBlt(
                    hdc,
                    1,
                    1,
                    st.thumb_w,
                    st.thumb_h,
                    Some(src),
                    0,
                    0,
                    st.img_w,
                    st.img_h,
                    SRCCOPY,
                );
                SelectObject(src, old);
                let _ = DeleteDC(src);
                // Hint bar.
                let bar = RECT {
                    left: 1,
                    top: 1 + st.thumb_h,
                    right: rc.right - 1,
                    bottom: rc.bottom - 1,
                };
                let bar_br = CreateSolidBrush(COLORREF(0x00F5F2EF));
                FillRect(hdc, &bar, bar_br);
                let _ = DeleteObject(bar_br.into());
                SetBkMode(hdc, TRANSPARENT);
                SetTextColor(hdc, COLORREF(0x00585858));
                let of = SelectObject(hdc, st.font.into());
                let mut text = wide("drag into app · double-click edits · right-click closes");
                let mut trc = bar;
                trc.left += 6;
                trc.right -= 6;
                let _ = DrawTextW(
                    hdc,
                    &mut text,
                    &mut trc,
                    DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_END_ELLIPSIS,
                );
                SelectObject(hdc, of);
            }
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            if let Some(st) = state(hwnd) {
                st.press = Some(POINT { x: x_lparam(lparam), y: y_lparam(lparam) });
                SetCapture(hwnd);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if let Some(st) = state(hwnd) {
                if let Some(p) = st.press {
                    let dx = (x_lparam(lparam) - p.x).abs();
                    let dy = (y_lparam(lparam) - p.y).abs();
                    let tx = GetSystemMetrics(SM_CXDRAG).max(4);
                    let ty = GetSystemMetrics(SM_CYDRAG).max(4);
                    if dx > tx || dy > ty {
                        st.press = None;
                        let _ = ReleaseCapture();
                        let path = st.path.clone();
                        if do_drag(hwnd, &path) {
                            let _ = DestroyWindow(hwnd);
                        }
                    }
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let _ = ReleaseCapture();
            if let Some(st) = state(hwnd) {
                st.press = None;
            }
            LRESULT(0)
        }
        WM_LBUTTONDBLCLK => {
            if let Some(st) = state(hwnd) {
                crate::open_editor(st.path.clone());
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 as u16 == VK_ESCAPE.0 {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_NCDESTROY => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ThumbState;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if !ptr.is_null() {
                let st = Box::from_raw(ptr);
                let _ = DeleteObject(st.hbmp.into());
                if !st.font.is_invalid() {
                    let _ = DeleteObject(st.font.into());
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
