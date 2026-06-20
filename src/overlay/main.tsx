import React from "react";
import ReactDOM from "react-dom/client";
import RecordingOverlay from "./RecordingOverlay";
import "@/i18n";

// Fonts — the overlay is its own window and loads Inter independently of the
// main app entry point.
import "@fontsource/inter/400.css";
import "@fontsource/inter/500.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <RecordingOverlay />
  </React.StrictMode>,
);
