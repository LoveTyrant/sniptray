use std::ffi::c_void;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub struct Screenshot {
    pub hbmp: HBITMAP,
    /// Virtual-screen origin (can be negative with multi-monitor setups).
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Drop for Screenshot {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.hbmp.into());
        }
    }
}

/// Grab the entire virtual screen (all monitors) into a bitmap.
pub fn capture_virtual_screen() -> Option<Screenshot> {
    unsafe {
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        if w <= 0 || h <= 0 {
            return None;
        }
        let sdc = GetDC(None);
        let mdc = CreateCompatibleDC(Some(sdc));
        let hbmp = CreateCompatibleBitmap(sdc, w, h);
        let old = SelectObject(mdc, hbmp.into());
        let rop = ROP_CODE(SRCCOPY.0 | CAPTUREBLT.0);
        let ok = BitBlt(mdc, 0, 0, w, h, Some(sdc), x, y, rop).is_ok();
        SelectObject(mdc, old);
        let _ = DeleteDC(mdc);
        ReleaseDC(None, sdc);
        if !ok {
            let _ = DeleteObject(hbmp.into());
            return None;
        }
        Some(Screenshot { hbmp, x, y, w, h })
    }
}

pub fn rect_contains(r: &RECT, p: POINT) -> bool {
    p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom
}

/// Clip a screen rect to the screenshot, in bitmap coordinates.
fn to_bitmap_rect(shot: &Screenshot, r: &RECT) -> RECT {
    RECT {
        left: (r.left - shot.x).clamp(0, shot.w),
        top: (r.top - shot.y).clamp(0, shot.h),
        right: (r.right - shot.x).clamp(0, shot.w),
        bottom: (r.bottom - shot.y).clamp(0, shot.h),
    }
}

/// Every monitor's rect, in the screenshot's bitmap coordinates.
pub fn monitor_rects(shot: &Screenshot) -> Vec<RECT> {
    unsafe extern "system" fn cb(_m: HMONITOR, _dc: HDC, r: *mut RECT, data: LPARAM) -> windows::core::BOOL {
        (*(data.0 as *mut Vec<RECT>)).push(*r);
        TRUE
    }
    let mut screen: Vec<RECT> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut screen as *mut _ as isize));
    }
    screen.iter().map(|r| to_bitmap_rect(shot, r)).collect()
}

/// Rects of the visible top-level windows, front to back, in the
/// screenshot's bitmap coordinates. Call before the overlay exists.
pub fn window_rects(shot: &Screenshot) -> Vec<RECT> {
    unsafe extern "system" fn cb(hwnd: HWND, data: LPARAM) -> windows::core::BOOL {
        let out = &mut *(data.0 as *mut Vec<RECT>);
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return TRUE;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == std::process::id() {
            return TRUE;
        }
        // Hidden UWP/virtual-desktop windows report as visible but cloaked.
        let mut cloaked = 0u32;
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut _ as *mut c_void,
            std::mem::size_of::<u32>() as u32,
        )
        .is_ok()
            && cloaked != 0
        {
            return TRUE;
        }
        // Click-through and fully transparent windows (e.g. GPU overlays
        // that cover the whole screen) would swallow every hover.
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex & WS_EX_TRANSPARENT.0 != 0 {
            return TRUE;
        }
        if ex & WS_EX_LAYERED.0 != 0 {
            let mut alpha = 255u8;
            let mut flags = LAYERED_WINDOW_ATTRIBUTES_FLAGS(0);
            if GetLayeredWindowAttributes(hwnd, None, Some(&mut alpha), Some(&mut flags)).is_ok()
                && flags.0 & LWA_ALPHA.0 != 0
                && alpha == 0
            {
                return TRUE;
            }
        }
        // The desktop itself: fall through to "whole monitor" instead.
        let mut class = [0u16; 32];
        let n = GetClassNameW(hwnd, &mut class) as usize;
        let class = String::from_utf16_lossy(&class[..n]);
        if class == "Progman" || class == "WorkerW" {
            return TRUE;
        }
        // The visible frame, without the invisible resize borders.
        let mut r = RECT::default();
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut _ as *mut c_void,
            std::mem::size_of::<RECT>() as u32,
        )
        .is_err()
            && GetWindowRect(hwnd, &mut r).is_err()
        {
            return TRUE;
        }
        if r.right - r.left > 1 && r.bottom - r.top > 1 {
            out.push(r);
        }
        TRUE
    }
    let mut screen: Vec<RECT> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut screen as *mut _ as isize));
    }
    screen
        .iter()
        .map(|r| to_bitmap_rect(shot, r))
        .filter(|r| r.right - r.left > 1 && r.bottom - r.top > 1)
        .collect()
}

/// A bitmap from top-down BGRA pixels.
pub fn bitmap_from_bgra(bgra: &[u8], w: i32, h: i32) -> Option<HBITMAP> {
    unsafe {
        let sdc = GetDC(None);
        let hbmp = CreateCompatibleBitmap(sdc, w, h);
        let mut bi = BITMAPINFO::default();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w;
        bi.bmiHeader.biHeight = -h; // top-down
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB.0;
        let n = SetDIBits(
            Some(sdc),
            hbmp,
            0,
            h as u32,
            bgra.as_ptr() as *const c_void,
            &bi,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, sdc);
        if n == 0 {
            let _ = DeleteObject(hbmp.into());
            return None;
        }
        Some(hbmp)
    }
}

/// A copy of the screenshot darkened for the selection overlay backdrop.
pub fn make_dimmed(shot: &Screenshot) -> Option<HBITMAP> {
    unsafe {
        let sdc = GetDC(None);
        let src = CreateCompatibleDC(Some(sdc));
        let dst = CreateCompatibleDC(Some(sdc));
        let bdc = CreateCompatibleDC(Some(sdc));
        let dim = CreateCompatibleBitmap(sdc, shot.w, shot.h);
        let black = CreateCompatibleBitmap(sdc, 1, 1);
        let o1 = SelectObject(src, shot.hbmp.into());
        let o2 = SelectObject(dst, dim.into());
        let o3 = SelectObject(bdc, black.into());

        let mut ok = BitBlt(dst, 0, 0, shot.w, shot.h, Some(src), 0, 0, SRCCOPY).is_ok();
        let br = CreateSolidBrush(COLORREF(0));
        let r = RECT { left: 0, top: 0, right: 1, bottom: 1 };
        FillRect(bdc, &r, br);
        let _ = DeleteObject(br.into());
        let bf = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 120,
            AlphaFormat: 0,
        };
        ok &= AlphaBlend(dst, 0, 0, shot.w, shot.h, bdc, 0, 0, 1, 1, bf).as_bool();

        SelectObject(src, o1);
        SelectObject(dst, o2);
        SelectObject(bdc, o3);
        let _ = DeleteDC(src);
        let _ = DeleteDC(dst);
        let _ = DeleteDC(bdc);
        let _ = DeleteObject(black.into());
        ReleaseDC(None, sdc);

        if ok {
            Some(dim)
        } else {
            let _ = DeleteObject(dim.into());
            None
        }
    }
}

/// Crop a region (bitmap coordinates) out of the screenshot into a new bitmap.
pub fn crop(shot: &Screenshot, sel: &RECT) -> Option<(HBITMAP, i32, i32)> {
    let w = sel.right - sel.left;
    let h = sel.bottom - sel.top;
    if w <= 0 || h <= 0 {
        return None;
    }
    unsafe {
        let sdc = GetDC(None);
        let src = CreateCompatibleDC(Some(sdc));
        let dst = CreateCompatibleDC(Some(sdc));
        let hcrop = CreateCompatibleBitmap(sdc, w, h);
        let o1 = SelectObject(src, shot.hbmp.into());
        let o2 = SelectObject(dst, hcrop.into());
        let ok = BitBlt(dst, 0, 0, w, h, Some(src), sel.left, sel.top, SRCCOPY).is_ok();
        SelectObject(src, o1);
        SelectObject(dst, o2);
        let _ = DeleteDC(src);
        let _ = DeleteDC(dst);
        ReleaseDC(None, sdc);
        if ok {
            Some((hcrop, w, h))
        } else {
            let _ = DeleteObject(hcrop.into());
            None
        }
    }
}

/// Top-down BGRA pixels of a bitmap, alpha forced opaque.
pub fn bgra_pixels(hbmp: HBITMAP, w: i32, h: i32) -> Option<Vec<u8>> {
    if w <= 0 || h <= 0 {
        return None;
    }
    unsafe {
        let sdc = GetDC(None);
        let mut bi = BITMAPINFO::default();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w;
        bi.bmiHeader.biHeight = -h; // top-down
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB.0;
        let mut buf = vec![0u8; (w as usize) * (h as usize) * 4];
        let n = GetDIBits(
            sdc,
            hbmp,
            0,
            h as u32,
            Some(buf.as_mut_ptr() as *mut c_void),
            &mut bi,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, sdc);
        if n == 0 {
            return None;
        }
        for px in buf.chunks_exact_mut(4) {
            px[3] = 0xFF;
        }
        Some(buf)
    }
}
