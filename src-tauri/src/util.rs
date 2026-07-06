use windows::Win32::Foundation::LPARAM;

/// UTF-16, null-terminated.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn x_lparam(l: LPARAM) -> i32 {
    (l.0 & 0xffff) as u16 as i16 as i32
}

pub fn y_lparam(l: LPARAM) -> i32 {
    ((l.0 >> 16) & 0xffff) as u16 as i16 as i32
}
