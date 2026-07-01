import React from "react";
import ReactDOM from "react-dom/client";
import AssistantPanel from "./AssistantPanel";
import { applyCachedAssistantTheme } from "@/lib/theme";
import "@/i18n";

// Fonts — the panel is its own window, so it must load fonts independently of
// the main app entry point. Plus Jakarta Sans (the brand face) carries it: body
// at 400, title/labels at 500, table headers at 600, markdown bold at 700.
import "@fontsource/plus-jakarta-sans/400.css";
import "@fontsource/plus-jakarta-sans/500.css";
import "@fontsource/plus-jakarta-sans/600.css";
import "@fontsource/plus-jakarta-sans/700.css";

// Apply the cached assistant-panel theme before render to avoid a theme flash.
// AssistantPanel re-applies the real setting (override or app-follow) on load.
applyCachedAssistantTheme();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <AssistantPanel />
  </React.StrictMode>,
);
