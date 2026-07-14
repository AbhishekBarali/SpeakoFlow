import React, { useState, useEffect } from "react";
import { getVersion } from "@tauri-apps/api/app";

import ModelSelector from "../model-selector";
import UpdateChecker from "../update-checker";

const Footer: React.FC = () => {
  const [version, setVersion] = useState("");

  useEffect(() => {
    const fetchVersion = async () => {
      try {
        const appVersion = await getVersion();
        setVersion(appVersion);
      } catch (error) {
        console.error("Failed to get app version:", error);
        setVersion("");
      }
    };

    fetchVersion();
  }, []);

  return (
    <div className="w-full border-t border-hairline bg-canvas-soft">
      {/* About and tray actions still use this controller's event listener, but
          update status no longer competes for permanent footer space. */}
      <UpdateChecker className="hidden" />
      <div className="flex items-center justify-between px-5 py-2.5 text-xs text-muted">
        <ModelSelector />
        {version && (
          <span className="tabular-nums text-muted-soft">{version}</span>
        )}
      </div>
    </div>
  );
};

export default Footer;
