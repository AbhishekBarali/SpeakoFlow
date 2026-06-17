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
        setVersion("0.1.2");
      }
    };

    fetchVersion();
  }, []);

  return (
    <div className="w-full border-t border-hairline bg-canvas-soft pt-3">
      <div className="flex justify-between items-center text-xs px-5 pb-3 text-muted">
        <div className="flex items-center gap-4">
          <ModelSelector />
        </div>

        {/* Update Status */}
        <div className="flex items-center gap-1.5">
          <UpdateChecker />
          <span className="text-muted-soft">•</span>
          {/* eslint-disable-next-line i18next/no-literal-string */}
          <span>v{version}</span>
        </div>
      </div>
    </div>
  );
};

export default Footer;
