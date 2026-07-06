use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::capture::{self, Screenshot};
use crate::util::{x_lparam, y_lparam};

static CLASS_REGISTERED: AtomicBool = AtomicBool::new(false);
const CLASS_NAME: PCWSTR = w!("SnipTrayOverlay");

struct OverlayState {
    shot: Screenshot,
    dim: HBITMAP,
    // Persistent double-buffer.
    buf_dc: HDC,
    buf_bmp: HBITMAP,
    buf_old: HGDIOBJ,
    dragging: bool,
    start: POINT,
    cur: POINT,
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
        let (x, y, w, h) = (shot.x, shot.y, shot.w, shot.h);
        let state = Box::new(OverlayState {
            shot,
            dim,
            buf_dc: HDC::default(),
            buf_bmp: HBITMAP::default(),
            buf_old: HGDIOBJ::default(),
            dragging: false,
            start: POINT::default(),
            cur: POINT::default(),
        });
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
                    let r = norm_rect(st.start, st.cur, w, h);
                    let rw = r.right - r.left;
                    let rh = r.bottom - r.top;
                    if rw > 0 && rh > 0 {
                        // Bright (undimmed) selection.
                        let shot_dc = CreateCompatibleDC(Some(hdc));
                        let os = SelectObject(shot_dc, st.shot.hbmp.into());
                        let _ = BitBlt(
                            st.buf_dc, r.left, r.top, rw, rh, Some(shot_dc), r.left, r.top,
                            SRCCOPY,
                        );
                        SelectObject(shot_dc, os);
                        let _ = DeleteDC(shot_dc);

                        // Contrasting double border.
                        let dark = CreateSolidBrush(COLORREF(0x00202020));
                        let white = CreateSolidBrush(COLORREF(0x00FFFFFF));
                        let outer = RECT {
                            left: r.left - 2,
                            top: r.top - 2,
                            right: r.right + 2,
                            bottom: r.bottom + 2,
                        };
                        let inner = RECT {
                            left: r.left - 1,
                            top: r.top - 1,
                            right: r.right + 1,
                            bottom: r.bottom + 1,
                        };
                        FrameRect(st.buf_dc, &outer, dark);
                        FrameRect(st.buf_dc, &inner, white);
                        let _ = DeleteObject(dark.into());
                        let _ = DeleteObject(white.into());

                        // Size label.
                        let label = format!("{rw} × {rh}");
                        let mut wl: Vec<u16> = label.encode_utf16().collect();
                        let mut size = SIZE::default();
                        let _ = GetTextExtentPoint32W(st.buf_dc, &wl, &mut size);
                        let pad = 5;
                        let mut lx = r.left;
                        let mut ly = r.bottom + 8;
                        if ly + size.cy + pad * 2 > h {
                            ly = (r.top - size.cy - pad * 2 - 8).max(0);
                        }
                        lx = lx.clamp(0, (w - size.cx - pad * 2).max(0));
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
                if st.dragging {
                    st.cur = POINT { x: x_lparam(lparam), y: y_lparam(lparam) };
                    let _ = InvalidateRect(Some(hwnd), None, false);
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
                        let _ = ShowWindow(hwnd, SW_HIDE);
                        crate::snip::complete(&st.shot, r);
                    }
                    let _ = DestroyWindow(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_RBUTTONDOWN => {
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
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut OverlayState;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if !ptr.is_null() {
                let st = Box::from_raw(ptr);
                SelectObject(st.buf_dc, st.buf_old);
                let _ = DeleteObject(st.buf_bmp.into());
                let _ = DeleteDC(st.buf_dc);
                let _ = DeleteObject(st.dim.into());
                // st.shot dropped here (deletes its bitmap).
            }
            crate::OVERLAY_ACTIVE.store(false, Ordering::SeqCst);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
