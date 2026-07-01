import React from "react";
import ReactDOM from "react-dom/client";
import RecordingOverlay from "./RecordingOverlay";
import "@/i18n";

// Fonts — the overlay is its own window and loads the brand sans (Plus Jakarta
// Sans) independently of the main app entry point.
import "@fontsource/plus-jakarta-sans/400.css";
import "@fontsource/plus-jakarta-sans/500.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <RecordingOverlay />
  </React.StrictMode>,
);
