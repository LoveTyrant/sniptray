use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_Pictures, SHGetKnownFolderPath, KNOWN_FOLDER_FLAG};

pub static CONFIG: LazyLock<Mutex<Config>> = LazyLock::new(|| Mutex::new(Config::load()));

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    /// Global shortcut in tauri format, e.g. "Control+PrintScreen".
    pub hotkey: String,
    /// Empty = default (Pictures\Snips)
    pub save_dir: String,
    /// Also place the saved PNG's file path on the clipboard as text.
    /// Terminal apps (e.g. Claude Code) attach images from pasted paths,
    /// and terminals like Warp only paste text into TUIs.
    pub clipboard_path_text: bool,
    /// After dropping the preview onto one of `terminal_drop_processes`,
    /// type the file path into it. Those apps hijack file drops for their
    /// own UI, so the terminal (and any TUI in it, like Claude Code) never
    /// sees the drop otherwise.
    pub terminal_drop_assist: bool,
    /// Lower-case executable names that swallow file drops.
    pub terminal_drop_processes: Vec<String>,
    /// Show the preview thumbnail on the monitor the snip was taken on,
    /// instead of always on the primary monitor.
    pub thumb_on_snip_monitor: bool,
    /// Move snips older than this many days to the Recycle Bin; 0 = never.
    pub delete_after_days: u32,
    pub first_run: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            hotkey: "Control+Shift+KeyX".to_string(),
            save_dir: String::new(),
            clipboard_path_text: true,
            terminal_drop_assist: true,
            terminal_drop_processes: vec!["warp.exe".to_string()],
            thumb_on_snip_monitor: false,
            delete_after_days: 0,
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
