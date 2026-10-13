# SnipTray

A fast, lightweight screenshot tool for Windows that lives in the system tray.
Native Win32 capture core (instant, multi-monitor, per-monitor-DPI aware)
wrapped in a [Tauri 2](https://tauri.app) shell with a React UI for settings
and annotation.

## Download

Get the latest installer from the
[Releases page](https://github.com/LoveTyrant/sniptray/releases/latest) —
download `SnipTray_<version>_x64-setup.exe` and run it. It installs for your
Windows account only (no admin rights needed) and SnipTray starts in the
system tray. Windows 10 or 11, 64-bit.

> **"Windows protected your PC"?** The installer isn't code-signed yet, so
> SmartScreen may warn you. Click **More info → Run anyway**.

## Features

- **Global hotkey** (default `Ctrl+Shift+X`) or left-click the tray icon to snip
- **Region, window, or full screen** — the screen freezes and dims; drag a
  region, click a highlighted window, or press `F` for the whole monitor
- **Delayed snip** — 3 / 5 / 10 s from the tray menu, for open menus and hover
  states
- **Annotate** — double-click the preview to open the editor: pen,
  highlighter, arrow, rectangle, ellipse, blur, text, crop, resize, undo/redo,
  one-key tool shortcuts (P H A R E B T C); *Save & copy* writes the file and
  refreshes the clipboard, and closing with unsaved changes asks first
- **Drag the result anywhere** — a preview thumbnail appears above the
  taskbar clock; drag it into any window (Outlook, webmail, chat apps…) to
  drop the PNG file
- **Recent snips** — the last five, in the tray menu; clicking one copies it
  again and brings back its preview
- **Clipboard** — every snip is copied as an image (DIB + PNG), and optionally
  as the saved file's path in text form, so terminals can paste it too
- **Terminal-AI friendly** — paste into Claude Code (or similar TUI tools) and
  it picks the snip up from the pasted path, even in terminals like Warp that
  don't forward file drops or images to TUI apps
- **Terminal drop assist** — Warp intercepts file drops, so a dropped snip
  would never reach the TUI inside; SnipTray detects a drop onto Warp and
  types the file path into the terminal instead, so drag & drop works with
  Claude Code running in Warp (configurable: `terminal_drop_processes`)
- **Saved automatically** to `Pictures\Snips` as timestamped PNGs, with
  optional cleanup of old snips

## Usage

Press the hotkey (or left-click the tray icon). While selecting:

| Action | Snips |
|---|---|
| Drag | The selected region |
| Click (no drag) | The highlighted window — or the whole monitor over the desktop |
| `F` | The whole monitor under the cursor |
| `Esc` or right-click | Nothing (cancels) |

Then, on the preview thumbnail:

| Preview thumbnail action | Result |
|---|---|
| Drag it into another window | Drops the PNG file there |
| Double-click | Opens the annotation editor |
| Right-click | Closes it |

### Tray menu

Right-click the tray icon:

- **Snip now** — same as the hotkey
- **Snip full screen** — the monitor under the cursor; when picked from the
  menu that's the monitor whose taskbar you opened it from (usually the main
  one). For another monitor, use the hotkey and press `F` over it.
- **Delayed snip** — in 3, 5 or 10 seconds
- **Recent snips** — the last five, newest first
- **Help** — opens the user guide (`sniptray-help.html`, installed next to
  the app) in the default browser
- **Settings…**, **Open snips folder**, **Exit**

## Settings

Right-click the tray icon → **Settings…**:

- **Snip hotkey** — click the binding and press a new combination. Plain
  letters need a modifier; F-keys and PrtSc work alone. Combos owned by other
  apps are rejected, and combos most apps already use (undo, redo, copy,
  paste, save…) get a warning, since a global hotkey takes them away from
  every app. In SnipTray's own editor the hotkey still does what the editor
  would normally do with it (e.g. `Ctrl+Shift+Z` redo). If the hotkey can't
  be registered at startup, Settings opens with the reason and the tray
  tooltip says so; the tray icon still snips on click.
- **Start with Windows** — on by default
- **Preview on snipped monitor** — show the preview thumbnail on the monitor
  you snipped instead of the main monitor (off by default)
- **Copy file path as text** — the terminal-paste behavior described above
- **Terminal drop assist** — toggles the Warp drop-to-path behavior described
  above (the list of apps it applies to is the config file's
  `terminal_drop_processes`)
- **Save snips to** — **Browse…** for a native folder picker, or **Default** to
  reset to `Pictures\Snips`. An **Open folder** button reveals the current one.
- **Delete old snips** — never (default), after 7 / 30 / 90 days, or
  **Custom…** for any number of days (1–3650). Old snips go to the Recycle
  Bin, and only SnipTray's own `snip_….png` files are touched. Runs at
  startup, after each snip, and when the setting changes.

All changes apply immediately. Config lives at `%APPDATA%\SnipTray\config.json`.

## Building

Requires Rust (stable, MSVC toolchain), Node 18+, and the Visual Studio Build
Tools with the C++ workload.

```
npm install
npm run tauri build      # → src-tauri/target/release/bundle/nsis/SnipTray_<version>_x64-setup.exe
```

The installer is per-user (no admin prompt) and installs to
`%LOCALAPPDATA%\SnipTray`. Run it with `/S` for a silent install.

**After changing `src-tauri/icons/icon.ico`**, run
`cargo clean --release -p sniptray` (from `src-tauri`) first — cargo doesn't
notice the change and keeps the old icon embedded in the exe. The installer
and uninstaller use the same icon (`tauri.conf.json` → `bundle.windows.nsis`).

**Version** lives in `package.json`, `package-lock.json`,
`src-tauri/tauri.conf.json`, and `src-tauri/Cargo.toml` — keep them in sync.

**Releasing:** bump the version, commit, and push a matching tag
(`git tag v1.2.3 && git push origin v1.2.3`). The
[release workflow](.github/workflows/release.yml) runs the tests, builds the
installer on GitHub, and attaches it to a draft release — review the notes
and publish it from the Releases page.

**Tests:** `cargo test` (from `src-tauri`).

The app is single-instance — launching a second copy opens the Settings
window of the running one.

## Architecture

- `src-tauri/src/capture.rs` — GDI screen capture, monitor and window
  enumeration (for click-to-snip a window), bitmap helpers
- `src-tauri/src/overlay.rs` — the raw-Win32 selection overlay: region drag,
  window highlight, full-monitor (kept native for latency; no webview
  involved in the snip path)
- `src-tauri/src/snip.rs` — PNG encode/decode, save, multi-format clipboard
- `src-tauri/src/thumb.rs` — native preview window with OLE drag-out
  (`SHDoDragDrop` with the shell's file formats only — adding a text format
  makes Chromium-based targets treat the drop as a link) and the Warp drop
  assist
- `src-tauri/src/history.rs` — recent snips for the tray menu, pruning old
  snips to the Recycle Bin
- `src-tauri/src/main.rs` — Tauri shell: tray menu, global shortcut, IPC
  commands
- `src-tauri/src/config.rs` — settings model, defaults, JSON persistence
- `src/settings`, `src/editor` — React UIs
- `src-tauri/help/sniptray-help.html` — self-contained user guide, bundled
  as an installer resource (`tauri.conf.json` → `bundle.resources`). Keep it
  in step with user-facing changes.

## License

[MIT](LICENSE)
