# SnipTray

A lightweight snipping tool for Windows that lives in the system tray. Pure
Win32 via the [`windows`](https://crates.io/crates/windows) crate — a single
~280 KB exe with no runtime dependencies, no installer, no background services.

## Features

- **Global hotkey** (default `Ctrl+PrtSc`) or left-click the tray icon to snip
- **Drag to select** — the screen freezes and dims, drag over the area you
  want, and it snips the moment you release
- **Drag the result anywhere** — a preview thumbnail appears bottom-right;
  drag it into any window (chat apps, browsers, editors, terminals, Explorer)
  to drop the screenshot as a file
- **Clipboard too** — every snip is copied as an image (DIB + PNG formats)
  *and* as the saved file's path in text form, so `Ctrl+V` works everywhere —
  image-aware apps paste the picture, terminals paste the path
- **Terminal-AI friendly** — paste into Claude Code (or similar TUI tools) and
  it picks the snip up from the pasted path, even in terminals like Warp that
  don't forward file drops or images to TUI apps
- **Saved automatically** to `Pictures\Snips` as timestamped PNGs
- **Rebindable hotkey** and **start-with-Windows** toggle, right in the tray menu
- Multi-monitor and per-monitor-DPI aware

## Usage

Press the hotkey (or left-click the tray icon), drag, release. Then:

| Preview thumbnail action | Result |
|---|---|
| Drag it into another window | Drops the PNG file there |
| Right-click | Closes it |
| Double-click | Opens the PNG |

While selecting, `Esc` or right-click cancels.

**Tray menu** (right-click the icon): *Snip now* · *Settings…* · *Open snips
folder* · *Exit*.

## Settings

Right-click the tray icon → **Settings…**:

- **Snip hotkey** — shows the current binding; *Change…* opens a capture
  window: press any combination. Plain letters need a modifier
  (Ctrl/Alt/Shift/Win); F-keys and PrtSc also work alone. Combinations owned
  by other apps are rejected with a message.
- **Start with Windows** — registers/unregisters the app in the user Run key.
- **Also copy file path as text** — puts the saved PNG's path on the clipboard
  as text alongside the image, so terminal-hosted tools (Claude Code etc.) can
  pick the snip up from a plain paste. Untick it if an app you paste into
  prefers the text over the image.
- **Save snips to** — where PNGs are stored (*Default* = `Pictures\Snips`).

All changes apply immediately. Settings persist in
`%APPDATA%\SnipTray\config.json`.

The *Start with Windows* toggle writes an `HKCU\...\Run` entry pointing at the
exe's current location; re-toggle it if you move the exe.

## Building

```
cargo build --release
```

Output: `target\release\sniptray.exe`. Requires stable Rust on Windows.

The app is single-instance — launching a second copy is a no-op. Exit the
running instance from the tray before rebuilding, or the linker can't
overwrite the exe.

`sniptray.exe --test-capture out.png` is a hidden self-test that captures the
full virtual screen to a PNG and exits.

## License

[MIT](LICENSE)
