import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { platform } from "@tauri-apps/plugin-os";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Copy, Minus, Square, X } from "lucide-react";
import Wordmark from "./Wordmark";

/**
 * Custom window title bar.
 *
 * The native window chrome is disabled on Windows/Linux (see the
 * `WebviewWindowBuilder` in `src-tauri/src/lib.rs`), so this bar carries the
 * brand and window controls and doubles as the drag region. On macOS the
 * window keeps an overlay title bar, so the native traffic lights float over
 * the left side and we only reserve space for them here.
 *
 * Rendered on every screen (onboarding included) so the window can always be
 * moved, resized, maximized, and closed.
 */
export const TitleBar: React.FC = () => {
  const { t } = useTranslation();
  const isMac = platform() === "macos";
  const appWindow = React.useMemo(() => getCurrentWindow(), []);
  const [isMaximized, setIsMaximized] = useState(false);

  useEffect(() => {
    if (isMac) return;

    let active = true;
    const refreshMaximizedState = () => {
      void appWindow
        .isMaximized()
        .then((maximized) => {
          if (active) setIsMaximized(maximized);
        })
        .catch((e) => console.warn("Failed to read window state:", e));
    };

    refreshMaximizedState();
    const unlisten = appWindow.onResized(refreshMaximizedState);

    return () => {
      active = false;
      void unlisten.then((stop) => stop());
    };
  }, [appWindow, isMac]);

  const handleMinimize = () => {
    appWindow
      .minimize()
      .catch((e) => console.warn("Failed to minimize window:", e));
  };

  const handleToggleMaximize = async () => {
    try {
      await appWindow.toggleMaximize();
      setIsMaximized(await appWindow.isMaximized());
    } catch (e) {
      console.warn("Failed to toggle maximize window:", e);
    }
  };

  const handleTitleBarDoubleClick = (event: React.MouseEvent) => {
    if (isMac || (event.target as HTMLElement).closest("button")) return;
    void handleToggleMaximize();
  };

  const handleClose = () => {
    // The backend intercepts close and hides the window to the tray
    // (see `on_window_event` in lib.rs), matching the native button.
    appWindow.close().catch((e) => console.warn("Failed to close window:", e));
  };

  return (
    <div
      data-tauri-drag-region
      onDoubleClick={handleTitleBarDoubleClick}
      className={`relative z-20 flex items-center justify-between h-11 shrink-0 bg-canvas-soft ${
        isMac ? "pl-[78px] pr-3" : "ps-3.5 pe-2"
      }`}
    >
      {/* Brand wordmark. pointer-events-none keeps the strip draggable. */}
      <div className="flex items-center pointer-events-none select-none">
        <Wordmark className="text-base" />
      </div>

      {/* Windows/Linux use these controls; macOS keeps native traffic lights. */}
      {!isMac && (
        <div className="flex items-center gap-0.5">
          <button
            type="button"
            aria-label={t("window.minimize")}
            title={t("window.minimize")}
            onClick={handleMinimize}
            className="flex items-center justify-center h-8 w-9 rounded-md text-muted hover:text-ink hover:bg-ink/8 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/70 transition-colors cursor-pointer"
          >
            <Minus width={16} height={16} />
          </button>
          <button
            type="button"
            aria-label={t(isMaximized ? "window.restore" : "window.maximize")}
            title={t(isMaximized ? "window.restore" : "window.maximize")}
            onClick={() => void handleToggleMaximize()}
            className="flex items-center justify-center h-8 w-9 rounded-md text-muted hover:text-ink hover:bg-ink/8 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/70 transition-colors cursor-pointer"
          >
            {isMaximized ? (
              <Copy width={13} height={13} />
            ) : (
              <Square width={13} height={13} />
            )}
          </button>
          <button
            type="button"
            aria-label={t("window.close")}
            title={t("window.close")}
            onClick={handleClose}
            className="flex items-center justify-center h-8 w-9 rounded-md text-muted hover:text-white hover:bg-error focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-error/70 transition-colors cursor-pointer"
          >
            <X width={16} height={16} />
          </button>
        </div>
      )}
    </div>
  );
};

export default TitleBar;
