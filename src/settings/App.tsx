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

  useEffect(() => {
    invoke<Config>("get_settings").then(setCfg);
    invoke<string>("effective_save_dir").then(setDir);
    isEnabled().then(setAutostart).catch(() => {});
  }, []);

  const flashSaved = () => {
    setSaved(true);
    window.setTimeout(() => setSaved(false), 1200);
  };

  const applyHotkeyEvent = async (e: React.KeyboardEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      setCapturing(false);
      setHkPreview("");
      setHkError("");
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
            }}
            onKeyDown={onHotkeyKeyDown}
            onKeyUp={onHotkeyKeyUp}
            onBlur={() => {
              setCapturing(false);
              setHkPreview("");
            }}
          >
            {capturing ? (hkPreview || "Press keys…") : pretty(cfg.hotkey)}
          </div>
        </div>
        {hkError && <p className="error">{hkError}</p>}
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
            <span className="label">Snips folder</span>
          </div>
          <button className="btn subtle" onClick={() => invoke("open_folder")}>
            Open folder
          </button>
        </div>
      </section>

      <footer>
        <button className="btn primary" onClick={() => invoke("snip_now")}>
          Snip now
        </button>
        <span className="tip">Tip: after a snip, paste with Ctrl+V — or drag the preview into any app.</span>
      </footer>
    </div>
  );
}
