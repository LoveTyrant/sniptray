# SnipTray

A fast, lightweight screenshot tool for Windows that lives in the system tray.
Native Win32 capture core (instant, multi-monitor, per-monitor-DPI aware)
wrapped in a [Tauri 2](https://tauri.app) shell with a React UI for settings
and annotation.

## Features

- **Global hotkey** (default `Ctrl+PrtSc`) or left-click the tray icon to snip
- **Drag to select** — the screen freezes and dims; it snips when you release
- **Annotate** — double-click the preview to open the editor: pen, arrow,
  rectangle, ellipse, blur, text, undo/redo; *Save & copy* writes the file and
  refreshes the clipboard
- **Drag the result anywhere** — a preview thumbnail appears above the
  taskbar clock; drag it into any window to drop the PNG file
- **Clipboard, three ways** — every snip is copied as an image (DIB + PNG)
  *and* as the saved file's path in text form, so `Ctrl+V` works everywhere:
  image apps paste the picture, terminals paste the path
- **Terminal-AI friendly** — paste into Claude Code (or similar TUI tools) and
  it picks the snip up from the pasted path, even in terminals like Warp that
  don't forward file drops or images to TUI apps
- **Terminal drop assist** — Warp intercepts file drops, so a dropped snip
  would never reach the TUI inside; SnipTray detects a drop onto Warp and
  types the file path into the terminal instead, so drag & drop works with
  Claude Code running in Warp (configurable: `terminal_drop_processes`)
- **Saved automatically** to `Pictures\Snips` as timestamped PNGs

## Usage

Press the hotkey (or left-click the tray icon), drag, release. Then:

| Preview thumbnail action | Result |
|---|---|
| Drag it into another window | Drops the PNG file there |
| Double-click | Opens the annotation editor |
| Right-click | Closes it |

While selecting, `Esc` or right-click cancels.

## Settings

Right-click the tray icon → **Settings…**:

- **Snip hotkey** — click the binding and press a new combination. Plain
  letters need a modifier; F-keys and PrtSc work alone. Combos owned by other
  apps are rejected.
- **Start with Windows**
- **Copy file path as text** — the terminal-paste behavior described above
- **Save snips to** — with a native folder picker

All changes apply immediately. Config lives at `%APPDATA%\SnipTray\config.json`.

## Building

Requires Rust (stable) and Node 18+.

```
npm install
npm run build            # frontend → dist/
cd src-tauri
cargo build --release    # → src-tauri/target/release/sniptray.exe
```

Or with the Tauri CLI for a bundled NSIS installer:

```
npx tauri build          # → src-tauri/target/release/bundle/nsis/*.exe
```

The app is single-instance — launching a second copy opens the Settings
window of the running one.

## Architecture

- `src-tauri/src/capture.rs`, `overlay.rs` — GDI screen capture and the
  raw-Win32 selection overlay (kept native for latency; no webview involved
  in the snip path)
- `src-tauri/src/snip.rs` — PNG encode, save, multi-format clipboard
- `src-tauri/src/thumb.rs` — native preview window with OLE drag-out
  (`SHDoDragDrop` + CF_UNICODETEXT path rendition)
- `src-tauri/src/main.rs` — Tauri shell: tray, global shortcut, IPC commands
- `src/settings`, `src/editor` — React UIs

## License

[MIT](LICENSE)
