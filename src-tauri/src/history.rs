//! Recent snips (for the tray menu) and pruning of old ones.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use windows::core::PCWSTR;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Shell::{
    SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FO_DELETE,
    SHFILEOPSTRUCTW,
};

use crate::config;

/// Only files SnipTray named itself — the snips folder may be one the user
/// keeps other things in, and those must never be touched.
fn is_snip_file(p: &Path) -> bool {
    p.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("snip_") && n.ends_with(".png"))
}

/// Snip files in the snips folder with their modified times, newest first.
fn snip_files() -> Vec<(PathBuf, SystemTime)> {
    let dir = config::snips_dir(&config::CONFIG.lock().unwrap());
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(PathBuf, SystemTime)> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| is_snip_file(p))
        .filter_map(|p| {
            let t = std::fs::metadata(&p).and_then(|m| m.modified()).ok()?;
            Some((p, t))
        })
        .collect();
    files.sort_by(|a, b| b.1.cmp(&a.1));
    files
}

pub fn recent(n: usize) -> Vec<PathBuf> {
    snip_files().into_iter().take(n).map(|(p, _)| p).collect()
}

/// Width and height from a PNG's IHDR chunk.
fn png_size(p: &Path) -> Option<(u32, u32)> {
    let mut head = [0u8; 24];
    std::fs::File::open(p).ok()?.read_exact(&mut head).ok()?;
    if &head[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(head[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(head[20..24].try_into().ok()?);
    Some((w, h))
}

/// Menu label for a snip, from its "snip_YYYYMMDD_HHMMSS_mmm.png" name:
/// "Today 13:05:12 — 800 × 600" or "Oct 12, 13:05 — 800 × 600".
pub fn label(p: &Path) -> String {
    let name = p.file_stem().and_then(|n| n.to_str()).unwrap_or_default();
    let parts: Vec<&str> = name.split('_').collect();
    let when = match parts.as_slice() {
        [_, date, time, ..] if date.len() == 8 && time.len() == 6 => {
            let st = unsafe { GetLocalTime() };
            let today = format!("{:04}{:02}{:02}", st.wYear, st.wMonth, st.wDay);
            let (hh, mm, ss) = (&time[0..2], &time[2..4], &time[4..6]);
            if *date == today {
                format!("Today {hh}:{mm}:{ss}")
            } else {
                const MONTHS: [&str; 12] = [
                    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov",
                    "Dec",
                ];
                let month = date[4..6].parse::<usize>().unwrap_or(1).clamp(1, 12);
                let day = date[6..8].trim_start_matches('0');
                format!("{} {day}, {hh}:{mm}", MONTHS[month - 1])
            }
        }
        _ => name.to_string(),
    };
    match png_size(p) {
        Some((w, h)) => format!("{when}  —  {w} × {h}"),
        None => when,
    }
}

/// Move snips older than the configured age to the Recycle Bin.
fn cleanup() {
    let days = config::CONFIG.lock().unwrap().delete_after_days;
    if days == 0 {
        return;
    }
    let cutoff = SystemTime::now() - Duration::from_secs(days as u64 * 24 * 60 * 60);
    let old: Vec<PathBuf> = snip_files()
        .into_iter()
        .filter(|(_, t)| *t < cutoff)
        .map(|(p, _)| p)
        .collect();
    if old.is_empty() {
        return;
    }
    // pFrom is a list of null-terminated paths ending in an extra null.
    let mut from: Vec<u16> = Vec::new();
    for p in &old {
        from.extend(p.to_string_lossy().encode_utf16());
        from.push(0);
    }
    from.push(0);
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: PCWSTR(from.as_ptr()),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI).0 as u16,
        ..Default::default()
    };
    unsafe {
        let _ = SHFileOperationW(&mut op);
    }
}

/// Prune old snips off the UI thread, then refresh the tray's Recent menu.
pub fn cleanup_async() {
    std::thread::spawn(|| {
        cleanup();
        crate::refresh_tray_menu();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_sniptray_files_are_snips() {
        assert!(is_snip_file(Path::new(r"C:\x\snip_20261013_130512_123.png")));
        assert!(!is_snip_file(Path::new(r"C:\x\holiday.png")));
        assert!(!is_snip_file(Path::new(r"C:\x\snip_notes.txt")));
        assert!(!is_snip_file(Path::new(r"C:\x\my_snip_1.png")));
    }

    #[test]
    fn label_formats_older_dates() {
        // No such file, so no size suffix.
        let l = label(Path::new(r"C:\nope\snip_20200307_090502_001.png"));
        assert_eq!(l, "Mar 7, 09:05");
    }

    #[test]
    fn label_falls_back_to_name() {
        assert_eq!(label(Path::new(r"C:\nope\snip_weird.png")), "snip_weird");
    }
}
