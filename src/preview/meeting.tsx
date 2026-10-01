// Must be first: installs the fake Tauri backend before any module touches it.
import { previewParams } from "./mockBackend";

/*
 * Browser preview of the floating meeting pill. Dev-only, like `main.tsx`.
 *
 * The pill is a window whose width Rust picks per mode and whose height is
 * whatever the webview measures, so this harness plays that part: it listens
 * for the `fit_meeting_pill` / `set_meeting_pill_expanded` calls the mock turns
 * into DOM events and sizes a frame to match, over a desktop-ish backdrop
 * (`&bg=light|dark|photo`) so the transparent edges can be judged.
 *
 * URL knobs: ?pill=collapsed|expanded|offer  &sysaudio=0  &paused=1
 */
import React, { useEffect, useState } from "react";
import ReactDOM from "react-dom/client";
import { emit } from "@tauri-apps/api/event";
import "@fontsource-variable/inter";
import "../i18n";
import MeetingPill from "@/meeting/MeetingPill";

const WIDTH = { collapsed: 152, offer: 304, expanded: 420 } as const;

const BACKDROPS: Record<string, string> = {
  light: "linear-gradient(135deg, #f3f4f6 0%, #e5e7eb 100%)",
  dark: "linear-gradient(135deg, #1f2937 0%, #111827 100%)",
  photo:
    "radial-gradient(circle at 20% 30%, #5b8def 0%, transparent 45%), radial-gradient(circle at 80% 70%, #f2a65a 0%, transparent 40%), #2c3e50",
};

const Harness: React.FC = () => {
  const mode = previewParams.get("pill") ?? "collapsed";
  const [expanded, setExpanded] = useState(mode === "expanded");
  const [height, setHeight] = useState<number>(36);

  useEffect(() => {
    const onPill = (event: Event) => {
      const detail = (
        event as CustomEvent<{
          height?: number;
          expanded?: boolean;
        }>
      ).detail;
      if (typeof detail.height === "number") setHeight(detail.height);
      if (typeof detail.expanded === "boolean") setExpanded(detail.expanded);
    };
    window.addEventListener("preview-pill", onPill);
    if (mode === "expanded") {
      window.setTimeout(() => void emit("meeting-pill-mode", true), 150);
    }
    return () => window.removeEventListener("preview-pill", onPill);
  }, [mode]);

  const width =
    mode === "offer"
      ? WIDTH.offer
      : expanded
        ? WIDTH.expanded
        : WIDTH.collapsed;

  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: BACKDROPS[previewParams.get("bg") ?? "light"],
      }}
    >
      <div
        data-preview-frame
        style={{
          position: "absolute",
          right: 24,
          bottom: 24,
          width,
          height: Math.min(Math.max(height, 30), 640),
          overflow: "hidden",
        }}
      >
        <MeetingPill />
      </div>
    </div>
  );
};

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Harness />
  </React.StrictMode>,
);
