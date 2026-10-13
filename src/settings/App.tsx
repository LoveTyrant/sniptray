import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { enable, disable, isEnabled } from "@tauri-apps/plugin-autostart";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

type Config = {
  hotkey: string;
  save_dir: string;
  clipboard_path_text: boolean;
  terminal_drop_assist: boolean;
  terminal_drop_processes: string[];
  thumb_on_snip_monitor: boolean;
  delete_after_days: number;
  first_run: boolean;
};

const pretty = (hk: string) =>
  hk
    .split("+")
    .map((t) =>
      t === "Control" ? "Ctrl"
      : t === "PrintScreen" ? "PrtSc"
      : t === "Super" ? "Win"
      : t.startsWith("Key") ? t.slice(3)
      : t.startsWith("Digit") ? t.slice(5)
      : t
    )
    .join(" + ");

// Combos other apps rely on. A global hotkey takes them away from every app
// while SnipTray runs, so warn (but allow it).
const COMMON_COMBOS: Record<string, string> = {
  "Control+KeyZ": "undo",
  "Control+Shift+KeyZ": "redo",
  "Control+KeyY": "redo",
  "Control+KeyC": "copy",
  "Control+KeyX": "cut",
  "Control+KeyV": "paste",
  "Control+KeyA": "select all",
  "Control+KeyS": "save",
  "Control+KeyF": "find",
  "Control+KeyP": "print",
};

const PRESET_DAYS = [0, 7, 30, 90];
const MAX_DAYS = 3650;

function Toggle({ checked, onChange }: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      className={`toggle ${checked ? "on" : ""}`}
      role="switch"
      aria-checked={checked}
      onClick={() => onChange(!checked)}
    >
      <span className="knob" />
    </button>
  );
}

export default function App() {
  const [cfg, setCfg] = useState<Config | null>(null);
  const [dir, setDir] = useState("");
  const [autostart, setAutostart] = useState(false);
  const [capturing, setCapturing] = useState(false);
  const [hkPreview, setHkPreview] = useState("");
  const [hkError, setHkError] = useState("");
  const [saved, setSaved] = useState(false);
  // "Delete old snips": custom mode shows a days field instead of a preset.
  const [customDays, setCustomDays] = useState(false);
  const [daysDraft, setDaysDraft] = useState("");
  const [daysError, setDaysError] = useState("");

  useEffect(() => {
    invoke<Config>("get_settings").then((c) => {
      setCfg(c);
      if (!PRESET_DAYS.includes(c.delete_after_days)) {
        setCustomDays(true);
        setDaysDraft(String(c.delete_after_days));
      }
    });
    invoke<string>("effective_save_dir").then(setDir);
    invoke<string | null>("get_hotkey_error").then((e) => e && setHkError(e));
    isEnabled().then(setAutostart).catch(() => {});
  }, []);

  const flashSaved = () => {
    setSaved(true);
    window.setTimeout(() => setSaved(false), 1200);
  };

  const saveDeleteAfterDays = async (v: number) => {
    await invoke("set_delete_after_days", { value: v });
    setCfg((c) => (c ? { ...c, delete_after_days: v } : c));
    flashSaved();
  };

  const commitCustomDays = async () => {
    const v = Number(daysDraft);
    if (!Number.isInteger(v) || v < 1 || v > MAX_DAYS) {
      setDaysError(`Enter a whole number of days from 1 to ${MAX_DAYS}.`);
      return;
    }
    setDaysError("");
    if (v !== cfg?.delete_after_days) await saveDeleteAfterDays(v);
  };

  const applyHotkeyEvent = async (e: React.KeyboardEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      setCapturing(false);
      setHkPreview("");
      setHkError("");
      invoke("end_hotkey_capture");
      return;
    }
    const mods: string[] = [];
    if (e.ctrlKey) mods.push("Control");
    if (e.altKey) mods.push("Alt");
    if (e.shiftKey) mods.push("Shift");
    if (e.metaKey) mods.push("Super");
    const code = e.code;
    if (/^(Control|Shift|Alt|Meta)(Left|Right)?$/.test(code)) {
      setHkPreview(mods.map((m) => pretty(m)).join(" + ") + " + …");
      setHkError("");
      return;
    }
    const standalone = /^F\d{1,2}$|^PrintScreen$|^Pause$|^ScrollLock$/.test(code);
    if (mods.length === 0 && !standalone) {
      setHkPreview(pretty(code));
      setHkError("Add Ctrl / Alt / Shift / Win — or use an F-key or PrtSc alone.");
      return;
    }
    const shortcut = [...mods, code].join("+");
    try {
      await invoke<string>("set_hotkey", { shortcut });
      setCfg((c) => (c ? { ...c, hotkey: shortcut } : c));
      setCapturing(false);
      setHkPreview("");
      setHkError("");
      flashSaved();
    } catch (err) {
      setHkError(String(err));
    }
  };

  const onHotkeyKeyDown = (e: React.KeyboardEvent) => {
    if (capturing) applyHotkeyEvent(e);
  };
  // Chromium only delivers keyup for PrintScreen.
  const onHotkeyKeyUp = (e: React.KeyboardEvent) => {
    if (capturing && e.code === "PrintScreen") applyHotkeyEvent(e);
  };

  if (!cfg) return null;

  return (
    <div className="page">
      <header>
        <div className="logo" />
        <div>
          <h1>SnipTray</h1>
          <p className="sub">Settings</p>
        </div>
        {saved && <span className="saved">Saved ✓</span>}
      </header>

      <section>
        <h2>Hotkey</h2>
        <div className="row">
          <div className="row-text">
            <span className="label">Snip hotkey</span>
            <span className="hint">Freezes the screen so you can drag over a region</span>
          </div>
          <div
            tabIndex={0}
            className={`hotkey ${capturing ? "capturing" : ""}`}
            onClick={() => {
              setCapturing(true);
              setHkPreview("");
              setHkError("");
              invoke("begin_hotkey_capture");
            }}
            onKeyDown={onHotkeyKeyDown}
            onKeyUp={onHotkeyKeyUp}
            onBlur={() => {
              setCapturing(false);
              setHkPreview("");
              invoke("end_hotkey_capture");
            }}
          >
            {capturing ? (hkPreview || "Press keys…") : pretty(cfg.hotkey)}
          </div>
        </div>
        {hkError && <p className="error">{hkError}</p>}
        {!hkError && !capturing && COMMON_COMBOS[cfg.hotkey] && (
          <p className="warn">
            {pretty(cfg.hotkey)} is {COMMON_COMBOS[cfg.hotkey]} in most apps — while SnipTray
            runs, it snips there instead.
          </p>
        )}
      </section>

      <section>
        <h2>General</h2>
        <div className="row">
          <div className="row-text">
            <span className="label">Start with Windows</span>
            <span className="hint">Launch SnipTray in the tray when you sign in</span>
          </div>
          <Toggle
            checked={autostart}
            onChange={async (v) => {
              try {
                if (v) await enable();
                else await disable();
                setAutostart(v);
                flashSaved();
              } catch {}
            }}
          />
        </div>
        <div className="row">
          <div className="row-text">
            <span className="label">Preview on snipped monitor</span>
            <span className="hint">
              Show the preview thumbnail on the monitor you snipped. Off: always
              on the main monitor.
            </span>
          </div>
          <Toggle
            checked={cfg.thumb_on_snip_monitor}
            onChange={async (v) => {
              await invoke("set_thumb_on_snip_monitor", { value: v });
              setCfg({ ...cfg, thumb_on_snip_monitor: v });
              flashSaved();
            }}
          />
        </div>
        <div className="row">
          <div className="row-text">
            <span className="label">Copy file path as text</span>
            <span className="hint">
              Lets terminal apps like Claude Code paste the snip. Turn off if an
              app pastes the path where you wanted the image.
            </span>
          </div>
          <Toggle
            checked={cfg.clipboard_path_text}
            onChange={async (v) => {
              await invoke("set_clipboard_path_text", { value: v });
              setCfg({ ...cfg, clipboard_path_text: v });
              flashSaved();
            }}
          />
        </div>
        <div className="row">
          <div className="row-text">
            <span className="label">Terminal drop assist</span>
            <span className="hint">
              Warp hijacks file drops, so a dropped snip never reaches the
              terminal. With this on, dropping onto Warp types the file path
              instead — Claude Code attaches it automatically.
            </span>
          </div>
          <Toggle
            checked={cfg.terminal_drop_assist}
            onChange={async (v) => {
              await invoke("set_terminal_drop_assist", { value: v });
              setCfg({ ...cfg, terminal_drop_assist: v });
              flashSaved();
            }}
          />
        </div>
      </section>

      <section>
        <h2>Storage</h2>
        <div className="row">
          <div className="row-text">
            <span className="label">Save snips to</span>
            <span className="hint dir">{dir}</span>
          </div>
          <div className="btn-group">
            <button
              className="btn"
              onClick={async () => {
                const picked = await openDialog({ directory: true });
                if (typeof picked === "string" && picked) {
                  const effective = await invoke<string>("set_save_dir", { dir: picked });
                  setDir(effective);
                  flashSaved();
                }
              }}
            >
              Browse…
            </button>
            <button
              className="btn subtle"
              onClick={async () => {
                const effective = await invoke<string>("set_save_dir", { dir: "" });
                setDir(effective);
                flashSaved();
              }}
            >
              Default
            </button>
          </div>
        </div>
        <div className="row">
          <div className="row-text">
            <span className="label">Delete old snips</span>
            <span className="hint">
              Moves snips older than this to the Recycle Bin. Only files SnipTray
              named (snip_….png) are touched.
            </span>
          </div>
          <div className="btn-group">
            <select
              className="select"
              value={customDays ? "custom" : cfg.delete_after_days}
              onChange={async (e) => {
                if (e.target.value === "custom") {
                  // Keep the current age until a custom one is entered.
                  setCustomDays(true);
                  setDaysDraft(String(cfg.delete_after_days || 14));
                  setDaysError("");
                  return;
                }
                setCustomDays(false);
                setDaysError("");
                await saveDeleteAfterDays(Number(e.target.value));
              }}
            >
              <option value={0}>Never</option>
              <option value={7}>After 7 days</option>
              <option value={30}>After 30 days</option>
              <option value={90}>After 90 days</option>
              <option value="custom">Custom…</option>
            </select>
            {customDays && (
              <input
                className="days"
                type="number"
                min={1}
                max={MAX_DAYS}
                value={daysDraft}
                aria-label="Days to keep snips"
                autoFocus
                onChange={(e) => setDaysDraft(e.target.value)}
                onBlur={commitCustomDays}
                onKeyDown={(e) => {
                  if (e.key === "Enter") (e.target as HTMLInputElement).blur();
                }}
              />
            )}
          </div>
        </div>
        {daysError && <p className="error">{daysError}</p>}
        <div className="row">
          <div className="row-text">
            <span className="label">Snips folder</span>
          </div>
          <button className="btn subtle" onClick={() => invoke("open_folder")}>
            Open folder
          </button>
        </div>
      </section>

      <footer>
        <span className="tip">Tip: after a snip, paste with Ctrl+V — or drag the preview into any app.</span>
      </footer>
    </div>
  );
}
