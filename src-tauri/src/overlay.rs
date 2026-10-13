use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::capture::{self, rect_contains, Screenshot};
use crate::util::{x_lparam, y_lparam};

static CLASS_REGISTERED: AtomicBool = AtomicBool::new(false);
const CLASS_NAME: PCWSTR = w!("SnipTrayOverlay");
const HINT: &str = "Drag to select  ·  Click a window  ·  F: full screen  ·  Esc: cancel";

struct OverlayState {
    shot: Screenshot,
    dim: HBITMAP,
    // Persistent double-buffer.
    buf_dc: HDC,
    buf_bmp: HBITMAP,
    buf_old: HGDIOBJ,
    font: HFONT,
    dragging: bool,
    start: POINT,
    cur: POINT,
    /// Snapshot of the desktop when the overlay opened (bitmap coords).
    windows: Vec<RECT>,
    monitors: Vec<RECT>,
    /// What a click (without dragging) would snip: the window under the
    /// cursor, or its monitor if there is none.
    hover: Option<RECT>,
}

impl OverlayState {
    fn target_at(&self, p: POINT) -> Option<RECT> {
        self.windows
            .iter()
            .find(|r| rect_contains(r, p))
            .or_else(|| self.monitors.iter().find(|r| rect_contains(r, p)))
            .copied()
    }

    fn monitor_at(&self, p: POINT) -> RECT {
        self.monitors
            .iter()
            .find(|r| rect_contains(r, p))
            .copied()
            .unwrap_or(RECT { left: 0, top: 0, right: self.shot.w, bottom: self.shot.h })
    }
}

pub fn show(shot: Screenshot) -> Result<()> {
    unsafe {
        let hinstance: HINSTANCE = GetModuleHandleW(None)?.into();
        if !CLASS_REGISTERED.swap(true, Ordering::SeqCst) {
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinstance,
                lpszClassName: CLASS_NAME,
                hCursor: LoadCursorW(None, IDC_CROSS)?,
                ..Default::default()
            };
            RegisterClassW(&wc);
        }

        let Some(dim) = capture::make_dimmed(&shot) else {
            return Err(Error::from(E_FAIL));
        };
        // Enumerate windows before the overlay exists, so it isn't one of them.
        let windows = capture::window_rects(&shot);
        let monitors = capture::monitor_rects(&shot);
        let (x, y, w, h) = (shot.x, shot.y, shot.w, shot.h);
        let mut state = Box::new(OverlayState {
            shot,
            dim,
            buf_dc: HDC::default(),
            buf_bmp: HBITMAP::default(),
            buf_old: HGDIOBJ::default(),
            font: HFONT::default(),
            dragging: false,
            start: POINT::default(),
            cur: POINT::default(),
            windows,
            monitors,
            hover: None,
        });
        let mut pt = POINT::default();
        if GetCursorPos(&mut pt).is_ok() {
            state.hover = state.target_at(POINT { x: pt.x - x, y: pt.y - y });
        }
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            CLASS_NAME,
            w!("SnipTray"),
            WS_POPUP,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(hinstance),
            Some(Box::into_raw(state) as *const _),
        )?;
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        Ok(())
    }
}

unsafe fn state<'a>(hwnd: HWND) -> Option<&'a mut OverlayState> {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut OverlayState;
    ptr.as_mut()
}

fn norm_rect(a: POINT, b: POINT, w: i32, h: i32) -> RECT {
    let mut r = RECT {
        left: a.x.min(b.x),
        top: a.y.min(b.y),
        right: a.x.max(b.x),
        bottom: a.y.max(b.y),
    };
    r.left = r.left.clamp(0, w);
    r.right = r.right.clamp(0, w);
    r.top = r.top.clamp(0, h);
    r.bottom = r.bottom.clamp(0, h);
    r
}

/// Text on a dark pill at (x, y), clamped inside the overlay.
unsafe fn draw_pill(st: &OverlayState, text: &str, x: i32, y: i32) {
    let mut wl: Vec<u16> = text.encode_utf16().collect();
    let mut size = SIZE::default();
    let _ = GetTextExtentPoint32W(st.buf_dc, &wl, &mut size);
    let pad = 6;
    let lx = x.clamp(0, (st.shot.w - size.cx - pad * 2).max(0));
    let ly = y.clamp(0, (st.shot.h - size.cy - pad * 2).max(0));
    let bg = RECT {
        left: lx,
        top: ly,
        right: lx + size.cx + pad * 2,
        bottom: ly + size.cy + pad * 2,
    };
    let bgbr = CreateSolidBrush(COLORREF(0x00202020));
    FillRect(st.buf_dc, &bg, bgbr);
    let _ = DeleteObject(bgbr.into());
    SetBkMode(st.buf_dc, TRANSPARENT);
    SetTextColor(st.buf_dc, COLORREF(0x00FFFFFF));
    let _ = TextOutW(st.buf_dc, lx + pad, ly + pad, &mut wl);
}

/// Undimmed region with a contrasting border and a size label.
unsafe fn draw_highlight(st: &OverlayState, ref_dc: HDC, r: RECT) {
    let rw = r.right - r.left;
    let rh = r.bottom - r.top;
    if rw <= 0 || rh <= 0 {
        return;
    }
    let shot_dc = CreateCompatibleDC(Some(ref_dc));
    let os = SelectObject(shot_dc, st.shot.hbmp.into());
    let _ = BitBlt(st.buf_dc, r.left, r.top, rw, rh, Some(shot_dc), r.left, r.top, SRCCOPY);
    SelectObject(shot_dc, os);
    let _ = DeleteDC(shot_dc);

    let dark = CreateSolidBrush(COLORREF(0x00202020));
    let white = CreateSolidBrush(COLORREF(0x00FFFFFF));
    let outer = RECT { left: r.left - 2, top: r.top - 2, right: r.right + 2, bottom: r.bottom + 2 };
    let inner = RECT { left: r.left - 1, top: r.top - 1, right: r.right + 1, bottom: r.bottom + 1 };
    FrameRect(st.buf_dc, &outer, dark);
    FrameRect(st.buf_dc, &inner, white);
    let _ = DeleteObject(dark.into());
    let _ = DeleteObject(white.into());

    // Size label below the region, or above it near the bottom edge.
    let label = format!("{rw} × {rh}");
    let mut size = SIZE::default();
    let wl: Vec<u16> = label.encode_utf16().collect();
    let _ = GetTextExtentPoint32W(st.buf_dc, &wl, &mut size);
    let mut ly = r.bottom + 8;
    if ly + size.cy + 12 > st.shot.h {
        ly = r.top - size.cy - 12 - 8;
    }
    draw_pill(st, &label, r.left, ly);
}

/// Hide the overlay, snip `r`, and close.
unsafe fn finish(hwnd: HWND, st: &OverlayState, r: RECT) {
    if r.right - r.left >= 3 && r.bottom - r.top >= 3 {
        let _ = ShowWindow(hwnd, SW_HIDE);
        crate::snip::complete(&st.shot, r);
    }
    let _ = DestroyWindow(hwnd);
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
                let wdc = GetDC(Some(hwnd));
                st.buf_dc = CreateCompatibleDC(Some(wdc));
                st.buf_bmp = CreateCompatibleBitmap(wdc, st.shot.w, st.shot.h);
                st.buf_old = SelectObject(st.buf_dc, st.buf_bmp.into());
                ReleaseDC(Some(hwnd), wdc);
                let dpi = GetDpiForWindow(hwnd) as i32;
                st.font = CreateFontW(
                    -(13 * dpi) / 96,
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
                SelectObject(st.buf_dc, st.font.into());
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            if let Some(st) = state(hwnd) {
                let (w, h) = (st.shot.w, st.shot.h);
                let dim_dc = CreateCompatibleDC(Some(hdc));
                let od = SelectObject(dim_dc, st.dim.into());
                let _ = BitBlt(st.buf_dc, 0, 0, w, h, Some(dim_dc), 0, 0, SRCCOPY);
                SelectObject(dim_dc, od);
                let _ = DeleteDC(dim_dc);

                if st.dragging {
                    draw_highlight(st, hdc, norm_rect(st.start, st.cur, w, h));
                } else {
                    if let Some(r) = st.hover {
                        draw_highlight(st, hdc, r);
                    }
                    // How-to hint, top-centre of every monitor.
                    let wl: Vec<u16> = HINT.encode_utf16().collect();
                    let mut size = SIZE::default();
                    let _ = GetTextExtentPoint32W(st.buf_dc, &wl, &mut size);
                    for m in &st.monitors {
                        let x = (m.left + m.right) / 2 - size.cx / 2 - 6;
                        draw_pill(st, HINT, x, m.top + 16);
                    }
                }
                let _ = BitBlt(hdc, 0, 0, w, h, Some(st.buf_dc), 0, 0, SRCCOPY);
            }
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            if let Some(st) = state(hwnd) {
                st.dragging = true;
                st.start = POINT { x: x_lparam(lparam), y: y_lparam(lparam) };
                st.cur = st.start;
                SetCapture(hwnd);
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if let Some(st) = state(hwnd) {
                let p = POINT { x: x_lparam(lparam), y: y_lparam(lparam) };
                if st.dragging {
                    st.cur = p;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                } else {
                    let t = st.target_at(p);
                    if t != st.hover {
                        st.hover = t;
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let _ = ReleaseCapture();
            if let Some(st) = state(hwnd) {
                if st.dragging {
                    st.dragging = false;
                    st.cur = POINT { x: x_lparam(lparam), y: y_lparam(lparam) };
                    let r = norm_rect(st.start, st.cur, st.shot.w, st.shot.h);
                    if r.right - r.left >= 3 && r.bottom - r.top >= 3 {
                        finish(hwnd, st, r);
                    } else if let Some(t) = st.target_at(st.cur) {
                        // A click, not a drag: snip the window (or monitor).
                        finish(hwnd, st, t);
                    } else {
                        let _ = DestroyWindow(hwnd);
                    }
                }
            }
            LRESULT(0)
        }
        WM_RBUTTONDOWN => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let key = wparam.0 as u16;
            if key == VK_ESCAPE.0 {
                let _ = DestroyWindow(hwnd);
            } else if key == VK_F.0 {
                if let Some(st) = state(hwnd) {
                    if !st.dragging {
                        let mut pt = POINT::default();
                        let _ = GetCursorPos(&mut pt);
                        let p = POINT { x: pt.x - st.shot.x, y: pt.y - st.shot.y };
                        let m = st.monitor_at(p);
                        finish(hwnd, st, m);
                    }
                }
            }
            LRESULT(0)
        }
        WM_NCDESTROY => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut OverlayState;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if !ptr.is_null() {
                let st = Box::from_raw(ptr);
                SelectObject(st.buf_dc, st.buf_old);
                let _ = DeleteObject(st.buf_bmp.into());
                let _ = DeleteDC(st.buf_dc);
                let _ = DeleteObject(st.dim.into());
                if !st.font.is_invalid() {
                    let _ = DeleteObject(st.font.into());
                }
                // st.shot dropped here (deletes its bitmap).
            }
            crate::OVERLAY_ACTIVE.store(false, Ordering::SeqCst);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
