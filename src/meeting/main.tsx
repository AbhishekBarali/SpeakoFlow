import React from "react";
import ReactDOM from "react-dom/client";
import MeetingPill from "./MeetingPill";
import "@/i18n";

// Its own window, so it loads its own font rather than inheriting one. Same
// family as everywhere else: an indicator that floats over the user's work in a
// different typeface reads as a different application.
import "@fontsource-variable/inter";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <MeetingPill />
  </React.StrictMode>,
);
