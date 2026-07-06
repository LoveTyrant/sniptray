import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

type Tool = "pen" | "arrow" | "rect" | "ellipse" | "blur" | "text";

type Pt = { x: number; y: number };

type Shape = {
  tool: Tool;
  color: string;
  width: number;
  points: Pt[]; // pen: many; others: [start, end]; text: [pos]
  text?: string;
};

const COLORS = ["#ef4444", "#f59e0b", "#22c55e", "#3b82f6", "#a855f7", "#ffffff", "#111111"];

export default function App() {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const imgRef = useRef<HTMLImageElement | null>(null);
  const [filePath, setFilePath] = useState("");
  const [loaded, setLoaded] = useState(false);
  const [tool, setTool] = useState<Tool>("arrow");
  const [color, setColor] = useState(COLORS[0]);
  const [width, setWidth] = useState(4);
  const [shapes, setShapes] = useState<Shape[]>([]);
  const [redoStack, setRedo] = useState<Shape[]>([]);
  const [live, setLive] = useState<Shape | null>(null);
  const [textEdit, setTextEdit] = useState<{ x: number; y: number; value: string } | null>(null);
  const [toast, setToast] = useState("");

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
        const c = canvasRef.current!;
        c.width = img.naturalWidth;
        c.height = img.naturalHeight;
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
    }
  };

  const redraw = useCallback(() => {
    const c = canvasRef.current;
    const img = imgRef.current;
    if (!c || !img) return;
    const ctx = c.getContext("2d")!;
    ctx.clearRect(0, 0, c.width, c.height);
    ctx.drawImage(img, 0, 0);
    for (const s of shapes) drawShape(ctx, s);
    if (live) drawShape(ctx, live);
  }, [shapes, live, loaded]);

  useEffect(() => {
    redraw();
  }, [redraw]);

  const canvasPos = (e: React.MouseEvent): Pt => {
    const c = canvasRef.current!;
    const r = c.getBoundingClientRect();
    return {
      x: ((e.clientX - r.left) / r.width) * c.width,
      y: ((e.clientY - r.top) / r.height) * c.height,
    };
  };

  const onDown = (e: React.MouseEvent) => {
    if (!loaded || textEdit) return;
    const p = canvasPos(e);
    if (tool === "text") {
      setTextEdit({ x: p.x, y: p.y, value: "" });
      return;
    }
    setLive({ tool, color, width, points: [p] });
  };

  const onMove = (e: React.MouseEvent) => {
    if (!live) return;
    const p = canvasPos(e);
    setLive((l) =>
      l ? { ...l, points: l.tool === "pen" ? [...l.points, p] : [l.points[0], p] } : l
    );
  };

  const onUp = () => {
    if (!live) return;
    if (live.points.length > 1) {
      setShapes((s) => [...s, live]);
      setRedo([]);
    }
    setLive(null);
  };

  const commitText = () => {
    if (!textEdit) return;
    if (textEdit.value.trim()) {
      setShapes((s) => [
        ...s,
        { tool: "text", color, width, points: [{ x: textEdit.x, y: textEdit.y }], text: textEdit.value },
      ]);
      setRedo([]);
    }
    setTextEdit(null);
  };

  const undo = () => {
    setShapes((s) => {
      if (!s.length) return s;
      setRedo((r) => [...r, s[s.length - 1]]);
      return s.slice(0, -1);
    });
  };

  const redoOp = () => {
    setRedo((r) => {
      if (!r.length) return r;
      setShapes((s) => [...s, r[r.length - 1]]);
      return r.slice(0, -1);
    });
  };

  const save = async () => {
    commitText();
    // Wait a tick so a pending text commit is drawn.
    requestAnimationFrame(async () => {
      redraw();
      const dataUrl = canvasRef.current!.toDataURL("image/png");
      try {
        await invoke("save_edited", { path: filePath, dataUrl });
        setToast("Saved & copied to clipboard");
        window.setTimeout(() => setToast(""), 1800);
      } catch (e) {
        setToast(`Save failed: ${e}`);
      }
    });
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
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
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const TOOLS: { id: Tool; label: string; glyph: string }[] = [
    { id: "pen", label: "Pen", glyph: "✏️" },
    { id: "arrow", label: "Arrow", glyph: "➚" },
    { id: "rect", label: "Rectangle", glyph: "▭" },
    { id: "ellipse", label: "Ellipse", glyph: "◯" },
    { id: "blur", label: "Blur region", glyph: "▒" },
    { id: "text", label: "Text", glyph: "T" },
  ];

  // Position of the floating text input in CSS pixels.
  const textEditCss = (() => {
    if (!textEdit || !canvasRef.current) return null;
    const c = canvasRef.current;
    const r = c.getBoundingClientRect();
    return {
      left: r.left + (textEdit.x / c.width) * r.width,
      top: r.top + (textEdit.y / c.height) * r.height,
    };
  })();

  return (
    <div className="editor">
      <div className="toolbar">
        <div className="tools">
          {TOOLS.map((t) => (
            <button
              key={t.id}
              title={t.label}
              className={`tool ${tool === t.id ? "active" : ""}`}
              onClick={() => setTool(t.id)}
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
        <button className="tool" title="Undo (Ctrl+Z)" onClick={undo} disabled={!shapes.length}>
          ↩
        </button>
        <button className="tool" title="Redo (Ctrl+Shift+Z)" onClick={redoOp} disabled={!redoStack.length}>
          ↪
        </button>
        <div className="grow" />
        {toast && <span className="toast">{toast}</span>}
        <button className="btn primary" onClick={save} title="Ctrl+S">
          Save & copy
        </button>
      </div>

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
    </div>
  );
}
