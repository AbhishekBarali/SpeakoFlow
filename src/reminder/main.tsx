import React from "react";
import ReactDOM from "react-dom/client";
import ReminderPopup from "./ReminderPopup";
import "@/i18n";

// Its own window, so it loads its own font rather than inheriting one. Same
// family as everywhere else: a reminder that arrives in a different typeface
// reads as a different application's notification.
import "@fontsource-variable/inter";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ReminderPopup />
  </React.StrictMode>,
);
