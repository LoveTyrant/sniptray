use std::ffi::c_void;
use std::io::Write as _;

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::{CF_DIB, CF_UNICODETEXT};
use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::{capture, config, history, thumb};

/// Called by the overlay once a region has been selected (bitmap coordinates).
pub fn complete(shot: &capture::Screenshot, sel: RECT) {
    let Some((hcrop, w, h)) = capture::crop(shot, &sel) else {
        return;
    };
    let Some(bgra) = capture::bgra_pixels(hcrop, w, h) else {
        unsafe { let _ = DeleteObject(hcrop.into()); }
        return;
    };
    let png = encode_png(&bgra, w, h);
    let path = png.as_deref().and_then(save_png);
    if let Some(ref png) = png {
        set_clipboard(&bgra, w, h, png, path.as_deref());
    }
    // Centre of the selection in screen coordinates.
    let at = POINT {
        x: shot.x + (sel.left + sel.right) / 2,
        y: shot.y + (sel.top + sel.bottom) / 2,
    };
    match path {
        Some(p) => thumb::show(hcrop, w, h, p, at),
        None => unsafe {
            let _ = DeleteObject(hcrop.into());
        },
    }
    // Prunes old snips (if enabled), then refreshes the Recent menu.
    history::cleanup_async();
}

/// Re-set the clipboard from edited PNG bytes (used by the editor's save).
pub fn reclip_png(png_bytes: &[u8], path: &str) -> Option<()> {
    let (bgra, w, h) = decode_png_bgra(png_bytes)?;
    set_clipboard(&bgra, w, h, png_bytes, Some(path));
    Some(())
}

/// Top-down BGRA pixels (alpha forced opaque) and size of a PNG.
pub fn decode_png_bgra(png_bytes: &[u8]) -> Option<(Vec<u8>, i32, i32)> {
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as i32, info.height as i32);
    let mut bgra = vec![0u8; (w as usize) * (h as usize) * 4];
    match info.color_type {
        png::ColorType::Rgba => {
            for (dst, src) in bgra.chunks_exact_mut(4).zip(buf.chunks_exact(4)) {
                dst[0] = src[2];
                dst[1] = src[1];
                dst[2] = src[0];
                dst[3] = 0xFF;
            }
        }
        png::ColorType::Rgb => {
            for (dst, src) in bgra.chunks_exact_mut(4).zip(buf.chunks_exact(3)) {
                dst[0] = src[2];
                dst[1] = src[1];
                dst[2] = src[0];
                dst[3] = 0xFF;
            }
        }
        _ => return None,
    }
    Some((bgra, w, h))
}

pub fn encode_png(bgra: &[u8], w: i32, h: i32) -> Option<Vec<u8>> {
    let mut rgba = bgra.to_vec();
    for px in rgba.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().ok()?;
        writer.write_image_data(&rgba).ok()?;
    }
    Some(out)
}

fn save_png(png: &[u8]) -> Option<String> {
    let dir = config::snips_dir(&config::CONFIG.lock().unwrap());
    std::fs::create_dir_all(&dir).ok()?;
    let st = unsafe { GetLocalTime() };
    let name = format!(
        "snip_{:04}{:02}{:02}_{:02}{:02}{:02}_{:03}.png",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond, st.wMilliseconds
    );
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).ok()?;
    f.write_all(png).ok()?;
    Some(path.to_string_lossy().into_owned())
}

/// Put the image on the clipboard as CF_DIB and "PNG", plus (optionally) the
/// saved file's path as CF_UNICODETEXT. The text rendition is what lets
/// terminal-hosted apps (Claude Code in Warp, etc.) pick the snip up from a
/// plain paste, since those hosts only forward text to TUI programs.
fn set_clipboard(bgra: &[u8], w: i32, h: i32, png: &[u8], path: Option<&str>) {
    unsafe {
        let mut opened = false;
        for _ in 0..10 {
            if OpenClipboard(None).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        if !opened {
            return;
        }
        let _ = EmptyClipboard();

        // CF_DIB: BITMAPINFOHEADER followed by bottom-up 32bpp rows.
        let stride = (w as usize) * 4;
        let mut dib = Vec::with_capacity(40 + bgra.len());
        let hdr = BITMAPINFOHEADER {
            biSize: 40,
            biWidth: w,
            biHeight: h, // bottom-up
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: bgra.len() as u32,
            ..Default::default()
        };
        dib.extend_from_slice(std::slice::from_raw_parts(
            &hdr as *const _ as *const u8,
            40,
        ));
        for row in (0..h as usize).rev() {
            dib.extend_from_slice(&bgra[row * stride..(row + 1) * stride]);
        }
        put_format(CF_DIB.0 as u32, &dib);

        let png_fmt = RegisterClipboardFormatW(w!("PNG"));
        if png_fmt != 0 {
            put_format(png_fmt, png);
        }

        if let Some(path) = path {
            let want_text = config::CONFIG.lock().unwrap().clipboard_path_text;
            if want_text {
                let w16: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
                let bytes =
                    std::slice::from_raw_parts(w16.as_ptr() as *const u8, w16.len() * 2);
                put_format(CF_UNICODETEXT.0 as u32, bytes);
            }
        }

        let _ = CloseClipboard();
    }
}

unsafe fn put_format(fmt: u32, bytes: &[u8]) {
    let Ok(hglobal) = GlobalAlloc(GMEM_MOVEABLE, bytes.len()) else {
        return;
    };
    let ptr = GlobalLock(hglobal);
    if ptr.is_null() {
        let _ = GlobalFree(Some(hglobal));
        return;
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
    let _ = GlobalUnlock(hglobal);
    if SetClipboardData(fmt, Some(HANDLE(hglobal.0 as *mut c_void))).is_err() {
        let _ = GlobalFree(Some(hglobal));
    }
}
