import React from "react";
import ReactDOM from "react-dom/client";
import AssistantPanel from "./AssistantPanel";
import "@/i18n";

// Fonts — the panel is its own window, so it must load fonts independently of
// the main app entry point. Inter (variable) carries it: body at 400,
// title/labels at 500, table headers at 600, markdown bold at 700.
import "@fontsource-variable/inter";

// The assistant window is dark-only (like the STT overlay) — no theme
// resolution needed before mount.

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <AssistantPanel />
  </React.StrictMode>,
);
