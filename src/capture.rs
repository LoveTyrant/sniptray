use std::ffi::c_void;

use windows::Win32::Foundation::*;
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
