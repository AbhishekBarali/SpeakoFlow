import React from "react";
import ReactDOM from "react-dom/client";
import AssistantPanel from "./AssistantPanel";
import { applyCachedTheme } from "@/lib/theme";
import "@/i18n";

// Fonts — Inter for UI/body, EB Garamond for the editorial display title.
// The panel is its own window, so it must load fonts independently of the
// main app entry point.
import "@fontsource/inter/400.css";
import "@fontsource/inter/500.css";
import "@fontsource/inter/600.css";
import "@fontsource/eb-garamond/400.css";
import "@fontsource/eb-garamond/500.css";

// Apply the cached appearance preference before render to avoid a theme flash.
// AssistantPanel re-applies the real setting once it loads.
applyCachedTheme();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <AssistantPanel />
  </React.StrictMode>,
);
