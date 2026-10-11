import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2, PlugZap } from "lucide-react";
import { commands } from "@/bindings";
import { SettingContainer } from "@/components/ui";
import { useSettingsStore } from "@/stores/settingsStore";
import { waitForSaves } from "@/lib/waitForSaves";

type TestState = "idle" | "testing" | "ok" | "error";

/**
 * Last row of the AI cleanup card: send one short request to whatever the rows
 * above resolve to, so a wrong key, an empty balance or a model id the endpoint
 * does not serve is found here instead of as a dictation pasted without
 * cleanup. Same check as the assistant's Test connection, against cleanup's own
 * provider and model.
 */
export const CleanupTestConnection: React.FC<{
  providerId: string;
  model: string;
  baseUrl: string;
  disabled?: boolean;
}> = ({ providerId, model, baseUrl, disabled = false }) => {
  const { t } = useTranslation();
  const [state, setState] = useState<TestState>("idle");
  const [message, setMessage] = useState<string | null>(null);

  // A result belongs to the provider, endpoint and model it was measured
  // against. Changing any of them clears it rather than leave a green line
  // vouching for a configuration nobody tested.
  useEffect(() => {
    setState("idle");
    setMessage(null);
  }, [providerId, model, baseUrl]);

  const handleTest = async () => {
    setState("testing");
    setMessage(null);
    try {
      // The fields above save on blur, and this click is what blurred them:
      // wait for those saves so a key pasted a moment ago is the one tested.
      // Each save marks its key as updating synchronously, so it is visible here.
      const keys = [
        "post_process_provider_id",
        `post_process_base_url:${providerId}`,
        `post_process_api_key:${providerId}`,
        `post_process_model:${providerId}`,
      ];
      await waitForSaves(
        () => keys.some((key) => useSettingsStore.getState().isUpdating[key]),
        useSettingsStore.subscribe,
      );
      const res = await commands.postProcessTestConnection();
      if (res.status === "error") {
        setState("error");
        setMessage(res.error);
        return;
      }
      setState("ok");
      setMessage(res.data);
    } catch (e) {
      setState("error");
      setMessage(String(e));
    }
  };

  return (
    <SettingContainer
      title={t("settings.postProcessing.api.testConnection.title")}
      description={t("settings.postProcessing.api.testConnection.description")}
      descriptionMode="tooltip"
      layout="horizontal"
      grouped={true}
    >
      <div className="flex flex-col items-end gap-1">
        <button
          type="button"
          onClick={() => void handleTest()}
          disabled={state === "testing" || disabled}
          className="flex items-center gap-2 px-3 py-1.5 rounded-lg border border-hairline-strong bg-surface hover:bg-surface-strong disabled:opacity-50 disabled:cursor-not-allowed text-[13px] font-medium cursor-pointer transition-colors"
        >
          {state === "testing" ? (
            <Loader2 size={14} className="animate-spin" />
          ) : (
            <PlugZap size={14} />
          )}
          {state === "testing"
            ? t("settings.postProcessing.api.testConnection.testing")
            : t("settings.postProcessing.api.testConnection.button")}
        </button>
        {message && (
          <span
            role="status"
            className={`text-xs max-w-[360px] text-right break-words ${
              state === "error" ? "text-error" : "text-muted-soft"
            }`}
          >
            {message}
          </span>
        )}
      </div>
    </SettingContainer>
  );
};
