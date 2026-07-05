use windows::Win32::Foundation::{LPARAM, WPARAM};

/// UTF-16, null-terminated.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn copy_wstr(dst: &mut [u16], s: &str) {
    let w: Vec<u16> = s.encode_utf16().collect();
    let n = w.len().min(dst.len().saturating_sub(1));
    dst[..n].copy_from_slice(&w[..n]);
    dst[n] = 0;
}

pub fn x_lparam(l: LPARAM) -> i32 {
    (l.0 & 0xffff) as u16 as i16 as i32
}

pub fn y_lparam(l: LPARAM) -> i32 {
    ((l.0 >> 16) & 0xffff) as u16 as i16 as i32
}

pub fn loword_w(w: WPARAM) -> u32 {
    (w.0 & 0xffff) as u32
}
