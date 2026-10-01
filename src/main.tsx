import React from "react";
import ReactDOM from "react-dom/client";
import { platform } from "@tauri-apps/plugin-os";
import App from "./App";
import { applyCachedTheme } from "./lib/theme";
import { watchScreenScale } from "./lib/screenScale";
import { suppressCaretBrowsing } from "./lib/caretBrowsing";
import { suppressBrowserContextMenu } from "./lib/contextMenu";

// No text caret in text that cannot be edited (see lib/caretBrowsing.ts).
suppressCaretBrowsing();
// No browser menu (Back / Refresh / Save as / Print) on right-click, except in
// text fields and on selected text (see lib/contextMenu.ts).
suppressBrowserContextMenu();

// Fonts — Inter (variable) carries every heading, label, control, and sentence;
// Instrument Serif is only the feature banners' headline (ui/Hero.tsx).
import "@fontsource-variable/inter";
import "@fontsource/instrument-serif/400.css";
import "@fontsource/instrument-serif/400-italic.css";

// Set platform before render so CSS can scope per-platform (e.g. scrollbar styles)
document.documentElement.dataset.platform = platform();

// Apply the cached appearance preference synchronously so the first paint uses
// the right palette (the real setting is re-applied once it loads in App.tsx).
applyCachedTheme();

// Size the rem scale for the display before the first paint (see
// lib/screenScale.ts), and keep it right if the window moves to another screen.
watchScreenScale();

// Initialize i18n
import "./i18n";

// Initialize model store (loads models and sets up event listeners)
import { useModelStore } from "./stores/modelStore";
useModelStore.getState().initialize();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
