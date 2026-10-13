import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

type Tool = "pen" | "highlight" | "arrow" | "rect" | "ellipse" | "blur" | "text" | "crop";

type Pt = { x: number; y: number };

type Shape = {
  tool: Tool;
  color: string;
  width: number;
  points: Pt[]; // pen/highlight: many; others: [start, end]; text: [pos]
  text?: string;
};

type Size = { w: number; h: number };
type Rect = { x: number; y: number; w: number; h: number };

// What the canvas shows: `view` is the region of the original image (in
// natural-image coordinates — crop shrinks it), drawn at `size` output pixels
// (resize changes it).
type Frame = { view: Rect; size: Size };

// One undoable step: a drawn shape, or a crop/resize. Shapes are stored in
// natural-image coordinates so a crop or resize never has to rewrite them —
// undoing one is just restoring the previous frame.
type Op =
  | { kind: "shape"; shape: Shape }
  | { kind: "frame"; from: Frame; to: Frame };

const COLORS = [
  "#ef4444",
  "#f59e0b",
  "#facc15",
  "#22c55e",
  "#3b82f6",
  "#a855f7",
  "#ffffff",
  "#111111",
];

const TOOLS: { id: Tool; label: string; glyph: string; key: string }[] = [
  { id: "pen", label: "Pen", glyph: "✏️", key: "P" },
  { id: "highlight", label: "Highlighter", glyph: "🖍", key: "H" },
  { id: "arrow", label: "Arrow", glyph: "➚", key: "A" },
  { id: "rect", label: "Rectangle", glyph: "▭", key: "R" },
  { id: "ellipse", label: "Ellipse", glyph: "◯", key: "E" },
  { id: "blur", label: "Blur region", glyph: "▒", key: "B" },
  { id: "text", label: "Text", glyph: "T", key: "T" },
  { id: "crop", label: "Crop", glyph: "⛶", key: "C" },
];

const MIN_PX = 8;

const sameOps = (a: Op[], b: Op[]) => a.length === b.length && a.every((o, i) => o === b[i]);

export default function App() {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const imgRef = useRef<HTMLImageElement | null>(null);
  const [filePath, setFilePath] = useState("");
  const [loaded, setLoaded] = useState(false);
  const [tool, setTool] = useState<Tool>("arrow");
  const [color, setColor] = useState(COLORS[0]);
  const [width, setWidth] = useState(4);
  const [ops, setOps] = useState<Op[]>([]);
  const [redoOps, setRedoOps] = useState<Op[]>([]);
  const [savedOps, setSavedOps] = useState<Op[]>([]);
  const [live, setLive] = useState<Shape | null>(null);
  const [textEdit, setTextEdit] = useState<{ x: number; y: number; value: string } | null>(null);
  const [frame, setFrame] = useState<Frame | null>(null);
  const [resizeUI, setResizeUI] = useState<{ w: string; h: string; lock: boolean } | null>(null);
  const [confirmClose, setConfirmClose] = useState(false);
  const [toast, setToast] = useState("");
  // The drawing tool to return to after a crop.
  const lastDrawTool = useRef<Tool>("arrow");

  const size = frame?.size ?? null;
  const shapes = ops.flatMap((o) => (o.kind === "shape" ? [o.shape] : []));
  const dirty = !sameOps(ops, savedOps) || !!textEdit?.value.trim();

  // Mirror of the state redraw needs, so the redraw() identity is stable and
  // deferred callers (save's rAF) always paint the latest state.
  const paintRef = useRef({ shapes, live, frame });
  paintRef.current = { shapes, live, frame };
  const opsRef = useRef(ops);
  opsRef.current = ops;
  const dirtyRef = useRef(dirty);
  dirtyRef.current = dirty;

  const flash = (msg: string, ms = 1800) => {
    setToast(msg);
    window.setTimeout(() => setToast(""), ms);
  };

  // Resolve which file this editor window is for.
  useEffect(() => {
    const label = getCurrentWebviewWindow().label;
    invoke<string | null>("get_editor_path", { label }).then((p) => {
      if (p) setFilePath(p);
    });
  }, []);

  // Load the image via IPC as a data URL — same-origin, so the canvas is
  // never tainted and toDataURL() keeps working for save.
  useEffect(() => {
    if (!filePath) return;
    invoke<string>("read_image", { path: filePath }).then((b64) => {
      const img = new Image();
      img.onload = () => {
        imgRef.current = img;
        const w = img.naturalWidth;
        const h = img.naturalHeight;
        setFrame({ view: { x: 0, y: 0, w, h }, size: { w, h } });
        setLoaded(true);
      };
      img.src = `data:image/png;base64,${b64}`;
    });
  }, [filePath]);

  const drawShape = (ctx: CanvasRenderingContext2D, s: Shape) => {
    ctx.strokeStyle = s.color;
    ctx.fillStyle = s.color;
    ctx.lineWidth = s.width;
    ctx.lineCap = "round";
    ctx.lineJoin = "round";
    const [a, b] = [s.points[0], s.points[s.points.length - 1]];
    switch (s.tool) {
      case "pen": {
        ctx.beginPath();
        s.points.forEach((p, i) => (i ? ctx.lineTo(p.x, p.y) : ctx.moveTo(p.x, p.y)));
        ctx.stroke();
        break;
      }
      case "highlight": {
        // One path stroked once, so overlapping segments don't darken.
        ctx.save();
        ctx.globalAlpha = 0.4;
        ctx.beginPath();
        s.points.forEach((p, i) => (i ? ctx.lineTo(p.x, p.y) : ctx.moveTo(p.x, p.y)));
        ctx.stroke();
        ctx.restore();
        break;
      }
      case "arrow": {
        ctx.beginPath();
        ctx.moveTo(a.x, a.y);
        ctx.lineTo(b.x, b.y);
        ctx.stroke();
        const ang = Math.atan2(b.y - a.y, b.x - a.x);
        const head = Math.max(12, s.width * 3.5);
        ctx.beginPath();
        ctx.moveTo(b.x, b.y);
        ctx.lineTo(b.x - head * Math.cos(ang - 0.45), b.y - head * Math.sin(ang - 0.45));
        ctx.moveTo(b.x, b.y);
        ctx.lineTo(b.x - head * Math.cos(ang + 0.45), b.y - head * Math.sin(ang + 0.45));
        ctx.stroke();
        break;
      }
      case "rect": {
        ctx.strokeRect(Math.min(a.x, b.x), Math.min(a.y, b.y), Math.abs(b.x - a.x), Math.abs(b.y - a.y));
        break;
      }
      case "ellipse": {
        ctx.beginPath();
        ctx.ellipse(
          (a.x + b.x) / 2,
          (a.y + b.y) / 2,
          Math.abs(b.x - a.x) / 2,
          Math.abs(b.y - a.y) / 2,
          0,
          0,
          Math.PI * 2
        );
        ctx.stroke();
        break;
      }
      case "blur": {
        const img = imgRef.current!;
        const x = Math.min(a.x, b.x);
        const y = Math.min(a.y, b.y);
        const w = Math.abs(b.x - a.x);
        const h = Math.abs(b.y - a.y);
        if (w < 2 || h < 2) break;
        ctx.save();
        ctx.beginPath();
        ctx.rect(x, y, w, h);
        ctx.clip();
        ctx.filter = "blur(14px)";
        ctx.drawImage(img, 0, 0);
        ctx.filter = "none";
        ctx.restore();
        break;
      }
      case "text": {
        const size = 14 + s.width * 5;
        ctx.font = `600 ${size}px "Segoe UI", sans-serif`;
        ctx.textBaseline = "top";
        const lines = (s.text ?? "").split("\n");
        lines.forEach((line, i) => {
          ctx.strokeStyle = "rgba(0,0,0,0.65)";
          ctx.lineWidth = Math.max(2, size / 8);
          ctx.strokeText(line, a.x, a.y + i * size * 1.25);
          ctx.fillText(line, a.x, a.y + i * size * 1.25);
        });
        break;
      }
      case "crop": {
        // Only ever live: dim everything outside the crop box.
        const img = imgRef.current!;
        const x = Math.min(a.x, b.x);
        const y = Math.min(a.y, b.y);
        const w = Math.abs(b.x - a.x);
        const h = Math.abs(b.y - a.y);
        ctx.save();
        ctx.fillStyle = "rgba(0,0,0,0.55)";
        ctx.beginPath();
        ctx.rect(0, 0, img.naturalWidth, img.naturalHeight);
        ctx.rect(x, y, w, h);
        ctx.fill("evenodd");
        ctx.strokeStyle = "#ffffff";
        ctx.lineCap = "butt";
        ctx.setLineDash([s.width * 4, s.width * 3]);
        ctx.strokeRect(x, y, w, h);
        ctx.restore();
        break;
      }
    }
  };

  const redraw = useCallback(() => {
    const { shapes, live, frame } = paintRef.current;
    const c = canvasRef.current;
    const img = imgRef.current;
    if (!c || !img || !frame) return;
    const { view, size } = frame;
    if (c.width !== size.w) c.width = size.w;
    if (c.height !== size.h) c.height = size.h;
    const ctx = c.getContext("2d")!;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, c.width, c.height);
    ctx.imageSmoothingQuality = "high";
    // Everything is stored in natural-image coordinates; the transform maps
    // the visible region (view) of the image and shapes onto the canvas.
    const sx = size.w / view.w;
    const sy = size.h / view.h;
    ctx.setTransform(sx, 0, 0, sy, -view.x * sx, -view.y * sy);
    ctx.drawImage(img, 0, 0);
    for (const s of shapes) drawShape(ctx, s);
    if (live) drawShape(ctx, live);
    ctx.setTransform(1, 0, 0, 1, 0, 0);
  }, []);

  useEffect(() => {
    redraw();
  }, [ops, live, frame, loaded, redraw]);

  const canvasPos = (e: React.MouseEvent): Pt => {
    const { view } = frame!;
    const r = canvasRef.current!.getBoundingClientRect();
    return {
      x: view.x + ((e.clientX - r.left) / r.width) * view.w,
      y: view.y + ((e.clientY - r.top) / r.height) * view.h,
    };
  };

  // On-screen output pixels → natural-image units. Shapes live in natural
  // coordinates, so new shapes bake in the current scale factor.
  const pxToNatural = (px: number) => {
    if (!frame) return px;
    const { view, size } = frame;
    return (px * (view.w / size.w + view.h / size.h)) / 2;
  };

  const selectTool = (t: Tool) => {
    if (t !== "crop") lastDrawTool.current = t;
    setTool(t);
  };

  const pushOp = (op: Op) => {
    setOps((h) => [...h, op]);
    setRedoOps([]);
  };

  const onDown = (e: React.MouseEvent) => {
    if (!loaded || textEdit) return;
    const p = canvasPos(e);
    if (tool === "text") {
      // The textarea mounts and autofocuses during this event's dispatch;
      // without preventDefault the mousedown's default action then moves
      // focus to the body, which blurs (and instantly closes) it.
      e.preventDefault();
      setTextEdit({ x: p.x, y: p.y, value: "" });
      return;
    }
    const w =
      tool === "crop"
        ? pxToNatural(2)
        : tool === "highlight"
          ? pxToNatural(Math.max(12, width * 4))
          : pxToNatural(width);
    setLive({ tool, color, width: w, points: [p] });
  };

  const onMove = (e: React.MouseEvent) => {
    if (!live) return;
    const p = canvasPos(e);
    setLive((l) =>
      l
        ? {
            ...l,
            points: l.tool === "pen" || l.tool === "highlight" ? [...l.points, p] : [l.points[0], p],
          }
        : l
    );
  };

  const applyCrop = (s: Shape) => {
    if (!frame) return;
    const { view, size } = frame;
    const [a, b] = [s.points[0], s.points[s.points.length - 1]];
    const x0 = Math.max(view.x, Math.min(a.x, b.x));
    const y0 = Math.max(view.y, Math.min(a.y, b.y));
    const x1 = Math.min(view.x + view.w, Math.max(a.x, b.x));
    const y1 = Math.min(view.y + view.h, Math.max(a.y, b.y));
    const crop = { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
    const out = {
      w: Math.round((crop.w * size.w) / view.w),
      h: Math.round((crop.h * size.h) / view.h),
    };
    if (out.w < MIN_PX || out.h < MIN_PX) return; // a click, not a crop
    const to = { view: crop, size: out };
    pushOp({ kind: "frame", from: frame, to });
    setFrame(to);
    setResizeUI(null);
    setTool(lastDrawTool.current);
  };

  const onUp = () => {
    if (!live) return;
    if (live.tool === "crop") {
      if (live.points.length > 1) applyCrop(live);
    } else if (live.points.length > 1) {
      pushOp({ kind: "shape", shape: live });
    }
    setLive(null);
  };

  const commitText = () => {
    if (!textEdit) return;
    if (textEdit.value.trim()) {
      pushOp({
        kind: "shape",
        shape: {
          tool: "text",
          color,
          width: pxToNatural(width),
          points: [{ x: textEdit.x, y: textEdit.y }],
          text: textEdit.value,
        },
      });
    }
    setTextEdit(null);
  };

  const undo = () => {
    if (!ops.length) return;
    const last = ops[ops.length - 1];
    setOps(ops.slice(0, -1));
    setRedoOps([...redoOps, last]);
    if (last.kind === "frame") setFrame(last.from);
  };

  const redoOp = () => {
    if (!redoOps.length) return;
    const last = redoOps[redoOps.length - 1];
    setRedoOps(redoOps.slice(0, -1));
    setOps([...ops, last]);
    if (last.kind === "frame") setFrame(last.to);
  };

  const openResize = () => {
    if (!size) return;
    setResizeUI(resizeUI ? null : { w: String(size.w), h: String(size.h), lock: true });
  };

  const onResizeInput = (field: "w" | "h", value: string) => {
    if (!resizeUI || !size) return;
    const next = { ...resizeUI, [field]: value };
    const n = Number(value);
    if (resizeUI.lock && Number.isFinite(n) && n > 0) {
      if (field === "w") next.h = String(Math.max(1, Math.round((n * size.h) / size.w)));
      else next.w = String(Math.max(1, Math.round((n * size.w) / size.h)));
    }
    setResizeUI(next);
  };

  const resizePreset = (pct: number) => {
    if (!resizeUI || !size) return;
    setResizeUI({
      ...resizeUI,
      w: String(Math.max(1, Math.round((size.w * pct) / 100))),
      h: String(Math.max(1, Math.round((size.h * pct) / 100))),
    });
  };

  const applyResize = () => {
    if (!resizeUI || !frame) return;
    const w = Math.round(Number(resizeUI.w));
    const h = Math.round(Number(resizeUI.h));
    if (!Number.isFinite(w) || !Number.isFinite(h) || w < MIN_PX || h < MIN_PX || w > 20000 || h > 20000) {
      flash(`Size must be between ${MIN_PX} and 20000 px`, 2500);
      return;
    }
    const to = { view: frame.view, size: { w, h } };
    pushOp({ kind: "frame", from: frame, to });
    setFrame(to);
    setResizeUI(null);
  };

  // Resolves true once the file is written and the clipboard updated.
  const save = () =>
    new Promise<boolean>((resolve) => {
      commitText();
      // Wait a tick so a pending text commit is drawn.
      requestAnimationFrame(async () => {
        redraw();
        const dataUrl = canvasRef.current!.toDataURL("image/png");
        try {
          await invoke("save_edited", { path: filePath, dataUrl });
          setSavedOps(opsRef.current);
          flash("Saved & copied to clipboard");
          resolve(true);
        } catch (e) {
          setToast(`Save failed: ${e}`);
          resolve(false);
        }
      });
    });

  // Closing with unsaved markup asks first.
  useEffect(() => {
    const un = getCurrentWebviewWindow().onCloseRequested((e) => {
      if (dirtyRef.current) {
        e.preventDefault();
        setConfirmClose(true);
      }
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  const closeNow = () => {
    getCurrentWebviewWindow().destroy();
  };

  const saveAndClose = async () => {
    setConfirmClose(false);
    if (await save()) closeNow();
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (confirmClose) {
        if (e.key === "Escape") setConfirmClose(false);
        if (e.key === "Enter") {
          e.preventDefault();
          saveAndClose();
        }
        return;
      }
      if (e.ctrlKey && e.code === "KeyZ") {
        e.preventDefault();
        e.shiftKey ? redoOp() : undo();
      }
      if (e.ctrlKey && e.code === "KeyY") {
        e.preventDefault();
        redoOp();
      }
      if (e.ctrlKey && e.code === "KeyS") {
        e.preventDefault();
        save();
      }
      // One-key tool shortcuts — not while typing.
      if (e.ctrlKey || e.altKey || e.metaKey || textEdit || resizeUI) return;
      const el = document.activeElement as HTMLInputElement | null;
      if (el?.tagName === "TEXTAREA" || (el?.tagName === "INPUT" && el.type !== "range")) return;
      const t = TOOLS.find((t) => `Key${t.key}` === e.code);
      if (t) {
        e.preventDefault();
        setLive(null);
        selectTool(t.id);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  // The snip hotkey is global, so Windows never delivers it to this window.
  // While the editor is focused the backend forwards it here instead
  // (e.g. "Control+Shift+KeyZ"); replay it as a keydown so it does what the
  // editor would normally do with it.
  useEffect(() => {
    const un = getCurrentWebviewWindow().listen<string>("hotkey-passthrough", (ev) => {
      const parts = ev.payload.split("+");
      window.dispatchEvent(
        new KeyboardEvent("keydown", {
          code: parts[parts.length - 1],
          ctrlKey: parts.includes("Control") || parts.includes("CommandOrControl"),
          shiftKey: parts.includes("Shift"),
          altKey: parts.includes("Alt"),
          metaKey: parts.includes("Super"),
          cancelable: true,
        })
      );
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  // Position of the floating text input in CSS pixels.
  const textEditCss = (() => {
    if (!textEdit || !canvasRef.current || !frame) return null;
    const { view } = frame;
    const r = canvasRef.current.getBoundingClientRect();
    return {
      left: r.left + ((textEdit.x - view.x) / view.w) * r.width,
      top: r.top + ((textEdit.y - view.y) / view.h) * r.height,
    };
  })();

  return (
    <div className="editor">
      <div className="toolbar">
        <div className="tools">
          {TOOLS.map((t) => (
            <button
              key={t.id}
              title={`${t.label} (${t.key})`}
              className={`tool ${tool === t.id ? "active" : ""}`}
              onClick={() => selectTool(t.id)}
            >
              {t.glyph}
            </button>
          ))}
        </div>
        <div className="sep" />
        <div className="colors">
          {COLORS.map((c) => (
            <button
              key={c}
              className={`swatch ${color === c ? "active" : ""}`}
              style={{ background: c }}
              onClick={() => setColor(c)}
            />
          ))}
        </div>
        <div className="sep" />
        <input
          type="range"
          min={1}
          max={12}
          value={width}
          onChange={(e) => setWidth(Number(e.target.value))}
          title="Stroke width"
        />
        <div className="sep" />
        <button className="tool" title="Undo (Ctrl+Z)" onClick={undo} disabled={!ops.length}>
          ↩
        </button>
        <button
          className="tool"
          title="Redo (Ctrl+Y or Ctrl+Shift+Z)"
          onClick={redoOp}
          disabled={!redoOps.length}
        >
          ↪
        </button>
        <div className="sep" />
        <button
          className={`tool ${resizeUI ? "active" : ""}`}
          title={size ? `Resize image (${size.w} × ${size.h})` : "Resize image"}
          onClick={openResize}
          disabled={!loaded}
        >
          ⤢
        </button>
        <div className="grow" />
        {toast && <span className="toast">{toast}</span>}
        <button className="btn primary" onClick={() => save()} title="Ctrl+S">
          Save & copy
        </button>
      </div>

      {resizeUI && size && (
        <div
          className="resize-pop"
          onKeyDown={(e) => {
            if (e.key === "Enter") applyResize();
            if (e.key === "Escape") setResizeUI(null);
          }}
        >
          <span className="dim">Resize</span>
          <input
            type="number"
            min={MIN_PX}
            value={resizeUI.w}
            onChange={(e) => onResizeInput("w", e.target.value)}
            title="Width (px)"
            autoFocus
          />
          <span className="dim">×</span>
          <input
            type="number"
            min={MIN_PX}
            value={resizeUI.h}
            onChange={(e) => onResizeInput("h", e.target.value)}
            title="Height (px)"
          />
          <button
            className={`tool ${resizeUI.lock ? "active" : ""}`}
            title="Lock aspect ratio"
            onClick={() => setResizeUI({ ...resizeUI, lock: !resizeUI.lock })}
          >
            🔗
          </button>
          <div className="sep" />
          {[25, 50, 75].map((pct) => (
            <button key={pct} className="btn" onClick={() => resizePreset(pct)}>
              {pct}%
            </button>
          ))}
          <div className="sep" />
          <button className="btn primary" onClick={applyResize}>
            Apply
          </button>
          <button className="btn" onClick={() => setResizeUI(null)}>
            Cancel
          </button>
        </div>
      )}

      <div className="canvas-wrap">
        <canvas
          ref={canvasRef}
          onMouseDown={onDown}
          onMouseMove={onMove}
          onMouseUp={onUp}
          onMouseLeave={onUp}
        />
        {textEdit && textEditCss && (
          <textarea
            autoFocus
            className="text-input"
            style={{ left: textEditCss.left, top: textEditCss.top, color }}
            value={textEdit.value}
            placeholder="Type… (Esc cancels)"
            onChange={(e) => setTextEdit({ ...textEdit, value: e.target.value })}
            onKeyDown={(e) => {
              if (e.key === "Escape") setTextEdit(null);
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                commitText();
              }
            }}
            onBlur={commitText}
          />
        )}
      </div>

      {confirmClose && (
        <div className="modal-backdrop">
          <div className="modal" role="dialog" aria-modal="true" aria-labelledby="close-title">
            <h2 id="close-title">Save changes to this snip?</h2>
            <p>Your markup will be lost if you close without saving.</p>
            <div className="modal-actions">
              <button className="btn" onClick={() => setConfirmClose(false)}>
                Cancel
              </button>
              <button className="btn" onClick={closeNow}>
                Don't save
              </button>
              <button className="btn primary" onClick={saveAndClose} autoFocus>
                Save & close
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
