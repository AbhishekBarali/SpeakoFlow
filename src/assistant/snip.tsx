import React, { useCallback, useEffect, useRef, useState } from "react";
import ReactDOM from "react-dom/client";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import "@/i18n";

/**
 * Region-snip overlay: a fullscreen transparent window where the user drags a
 * rectangle to screenshot part of the screen. The actual pixels come from a
 * frame frozen BEFORE this window opened (see assistant.rs), so the dimmer
 * never appears in the crop. Esc / right-click / a stray click cancels.
 */

interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

const Snip: React.FC = () => {
  const { t } = useTranslation();
  const [start, setStart] = useState<{ x: number; y: number } | null>(null);
  const [rect, setRect] = useState<Rect | null>(null);
  const finishedRef = useRef(false);

  const finish = useCallback(async (selection: Rect | null) => {
    if (finishedRef.current) return;
    finishedRef.current = true;
    try {
      await commands.assistantFinishRegionSnip(selection);
    } catch {
      // The window is closed by the backend either way.
    }
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") void finish(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [finish]);

  const onMouseDown = (e: React.MouseEvent) => {
    if (e.button === 2) {
      void finish(null);
      return;
    }
    setStart({ x: e.clientX, y: e.clientY });
    setRect(null);
  };

  const onMouseMove = (e: React.MouseEvent) => {
    if (!start) return;
    setRect({
      x: Math.min(start.x, e.clientX),
      y: Math.min(start.y, e.clientY),
      width: Math.abs(e.clientX - start.x),
      height: Math.abs(e.clientY - start.y),
    });
  };

  const onMouseUp = () => {
    if (rect && rect.width >= 4 && rect.height >= 4) {
      void finish(rect);
    } else {
      void finish(null);
    }
  };

  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        cursor: "crosshair",
      }}
      onMouseDown={onMouseDown}
      onMouseMove={onMouseMove}
      onMouseUp={onMouseUp}
      onContextMenu={(e) => {
        e.preventDefault();
        void finish(null);
      }}
    >
      {/* Dimmer with a punched-out selection: the rect stays clear while a
          huge box-shadow darkens everything around it. */}
      {rect ? (
        <div
          style={{
            position: "absolute",
            left: rect.x,
            top: rect.y,
            width: rect.width,
            height: rect.height,
            border: "1px solid rgba(255, 255, 255, 0.9)",
            boxShadow: "0 0 0 100000px rgba(10, 9, 8, 0.4)",
            pointerEvents: "none",
          }}
        />
      ) : (
        <div
          style={{
            position: "absolute",
            inset: 0,
            background: "rgba(10, 9, 8, 0.25)",
            pointerEvents: "none",
          }}
        />
      )}
      {!rect && (
        <div
          style={{
            position: "absolute",
            top: 28,
            left: "50%",
            transform: "translateX(-50%)",
            padding: "7px 14px",
            borderRadius: 999,
            background: "rgba(24, 22, 20, 0.92)",
            border: "1px solid rgba(255, 255, 255, 0.1)",
            color: "#f5f5f4",
            fontFamily:
              '"Plus Jakarta Sans", -apple-system, "Segoe UI", sans-serif',
            fontSize: 12.5,
            pointerEvents: "none",
          }}
        >
          {t("assistant.snip.hint")}
        </div>
      )}
    </div>
  );
};

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Snip />
  </React.StrictMode>,
);
