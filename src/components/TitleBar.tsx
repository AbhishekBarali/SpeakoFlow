import React from "react";
import { useTranslation } from "react-i18next";
import { platform } from "@tauri-apps/plugin-os";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, X } from "lucide-react";
import Wordmark from "./Wordmark";

/**
 * Custom window title bar.
 *
 * The native window chrome is disabled on Windows/Linux (see the
 * `WebviewWindowBuilder` in `src-tauri/src/lib.rs`), so this bar carries the
 * brand and the minimize / close controls and doubles as the drag region.
 * On macOS the window keeps an overlay title bar, so the native traffic lights
 * float over the left side and we only reserve space for them here.
 *
 * Rendered on every screen (onboarding included) so the window can always be
 * moved and closed.
 */
export const TitleBar: React.FC = () => {
  const { t } = useTranslation();
  const isMac = platform() === "macos";

  const handleMinimize = () => {
    getCurrentWindow()
      .minimize()
      .catch((e) => console.warn("Failed to minimize window:", e));
  };

  const handleClose = () => {
    // The backend intercepts close and hides the window to the tray
    // (see `on_window_event` in lib.rs), matching the native button.
    getCurrentWindow()
      .close()
      .catch((e) => console.warn("Failed to close window:", e));
  };

  return (
    <div
      data-tauri-drag-region
      className={`relative z-20 flex items-center justify-between h-11 shrink-0 bg-canvas-soft ${
        isMac ? "pl-[78px] pr-3" : "ps-3.5 pe-2"
      }`}
    >
      {/* Brand — wordmark only (logo tile removed while experimenting).
          pointer-events-none so the entire strip stays a drag handle. */}
      <div className="flex items-center pointer-events-none select-none">
        <Wordmark className="text-base" />
      </div>

      {/* Window controls — Windows/Linux only. macOS shows native traffic
          lights via the overlay title bar. No maximize: the window is not
          maximizable (see lib.rs). */}
      {!isMac && (
        <div className="flex items-center gap-0.5">
          <button
            type="button"
            aria-label={t("window.minimize")}
            title={t("window.minimize")}
            onClick={handleMinimize}
            className="flex items-center justify-center h-8 w-9 rounded-md text-muted hover:text-ink hover:bg-ink/8 transition-colors cursor-pointer"
          >
            <Minus width={16} height={16} />
          </button>
          <button
            type="button"
            aria-label={t("window.close")}
            title={t("window.close")}
            onClick={handleClose}
            className="flex items-center justify-center h-8 w-9 rounded-md text-muted hover:text-white hover:bg-error transition-colors cursor-pointer"
          >
            <X width={16} height={16} />
          </button>
        </div>
      )}
    </div>
  );
};

export default TitleBar;
