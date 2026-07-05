use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use windows::core::w;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::{FOLDERID_Pictures, SHGetKnownFolderPath, KNOWN_FOLDER_FLAG};

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    /// MOD_ALT=1 | MOD_CONTROL=2 | MOD_SHIFT=4 | MOD_WIN=8
    pub hotkey_mods: u32,
    pub hotkey_vk: u32,
    /// Empty = default (Pictures\Snips)
    pub save_dir: String,
    /// Also place the saved PNG's file path on the clipboard as text.
    /// Terminal apps (e.g. Claude Code) attach images from pasted paths,
    /// and terminals like Warp only paste text into TUIs.
    pub clipboard_path_text: bool,
    pub first_run: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            hotkey_mods: MOD_CONTROL.0,
            hotkey_vk: VK_SNAPSHOT.0 as u32,
            save_dir: String::new(),
            clipboard_path_text: true,
            first_run: true,
        }
    }
}

fn config_path() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join("SnipTray").join("config.json"))
}

impl Config {
    pub fn load() -> Config {
        config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(p) = config_path() {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(s) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(p, s);
            }
        }
    }

    pub fn hotkey_string(&self) -> String {
        hotkey_string(self.hotkey_mods, self.hotkey_vk)
    }
}

pub fn hotkey_string(mods: u32, vk: u32) -> String {
    let mut s = String::new();
    if mods & MOD_CONTROL.0 != 0 {
        s.push_str("Ctrl+");
    }
    if mods & MOD_ALT.0 != 0 {
        s.push_str("Alt+");
    }
    if mods & MOD_SHIFT.0 != 0 {
        s.push_str("Shift+");
    }
    if mods & MOD_WIN.0 != 0 {
        s.push_str("Win+");
    }
    s.push_str(&vk_name(vk));
    s
}

pub fn vk_name(vk: u32) -> String {
    match vk as u16 {
        0x30..=0x39 | 0x41..=0x5A => char::from_u32(vk).unwrap_or('?').to_string(),
        v if v >= VK_F1.0 && v <= VK_F24.0 => format!("F{}", v - VK_F1.0 + 1),
        v if v == VK_SNAPSHOT.0 => "PrtSc".to_string(),
        v if v == VK_SPACE.0 => "Space".to_string(),
        v if v == VK_PAUSE.0 => "Pause".to_string(),
        v if v == VK_SCROLL.0 => "ScrollLock".to_string(),
        v if v == VK_INSERT.0 => "Insert".to_string(),
        v if v == VK_DELETE.0 => "Delete".to_string(),
        v if v == VK_HOME.0 => "Home".to_string(),
        v if v == VK_END.0 => "End".to_string(),
        v if v == VK_PRIOR.0 => "PageUp".to_string(),
        v if v == VK_NEXT.0 => "PageDown".to_string(),
        v if v == VK_UP.0 => "Up".to_string(),
        v if v == VK_DOWN.0 => "Down".to_string(),
        v if v == VK_LEFT.0 => "Left".to_string(),
        v if v == VK_RIGHT.0 => "Right".to_string(),
        _ => {
            // Ask Windows for the key's display name.
            unsafe {
                let scan = MapVirtualKeyW(vk, MAPVK_VK_TO_VSC);
                let mut buf = [0u16; 64];
                let n = GetKeyNameTextW((scan << 16) as i32, &mut buf);
                if n > 0 {
                    String::from_utf16_lossy(&buf[..n as usize])
                } else {
                    format!("VK{vk:#04X}")
                }
            }
        }
    }
}

/// Default (or configured) folder where snips are saved.
pub fn snips_dir(cfg: &Config) -> PathBuf {
    if !cfg.save_dir.is_empty() {
        return PathBuf::from(&cfg.save_dir);
    }
    let pictures = unsafe {
        match SHGetKnownFolderPath(&FOLDERID_Pictures, KNOWN_FOLDER_FLAG(0), None) {
            Ok(pw) => {
                let s = pw.to_string().unwrap_or_default();
                CoTaskMemFree(Some(pw.0 as _));
                s
            }
            Err(_) => String::new(),
        }
    };
    if pictures.is_empty() {
        let home = std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into());
        PathBuf::from(home).join("Pictures").join("Snips")
    } else {
        PathBuf::from(pictures).join("Snips")
    }
}

const RUN_KEY: windows::core::PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: windows::core::PCWSTR = w!("SnipTray");

pub fn startup_enabled() -> bool {
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, Some(0), KEY_QUERY_VALUE, &mut hkey)
            != ERROR_SUCCESS
        {
            return false;
        }
        let r = RegQueryValueExW(hkey, RUN_VALUE, None, None, None, None);
        let _ = RegCloseKey(hkey);
        r == ERROR_SUCCESS
    }
}

pub fn set_startup(enable: bool) {
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            Some(0),
            KEY_SET_VALUE,
            &mut hkey,
        ) != ERROR_SUCCESS
        {
            return;
        }
        if enable {
            if let Ok(exe) = std::env::current_exe() {
                let cmd = format!("\"{}\"", exe.display());
                let w: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
                let bytes =
                    std::slice::from_raw_parts(w.as_ptr() as *const u8, w.len() * 2);
                let _ = RegSetValueExW(hkey, RUN_VALUE, None, REG_SZ, Some(bytes));
            }
        } else {
            let _ = RegDeleteValueW(hkey, RUN_VALUE);
        }
        let _ = RegCloseKey(hkey);
    }
}
