import React from "react";
import { useTranslation } from "react-i18next";
import { Check, Download, Loader2, X } from "lucide-react";
import { useModelStore } from "@/stores/modelStore";
import { SettingContainer } from "@/components/ui/SettingContainer";
import { Dropdown } from "@/components/ui/Dropdown";
import { Button } from "@/components/ui/Button";
import {
  KITTEN_SIZES,
  effectiveNativeVoice,
  kittenSizeFor,
  nativePackId,
  nativeVoices,
  type NativeEngine,
} from "@/lib/nativeVoices";

/**
 * Everything a native-only engine needs below the engine grid: for Kitten, which
 * size; then its download, and which voice. The size is the engine's model
 * setting, so each engine keeps its own like every other engine does.
 */
export const NativeEngineRows: React.FC<{
  engine: NativeEngine;
  /** The engine's model setting (Kitten's size, as a catalog id). */
  model: string;
  voice: string;
  unsupported: boolean;
  disabled: boolean;
  onModel: (model: string) => void;
  onVoice: (voice: string) => void;
}> = ({ engine, model, voice, unsupported, disabled, onModel, onVoice }) => {
  const { t } = useTranslation();
  const models = useModelStore((state) => state.models);
  const packId = nativePackId(engine, model)!;
  const sizeOf = (id: string) =>
    t("settings.assistant.tts.packSize", {
      size: models.find((m) => m.id === id)?.size_mb ?? "…",
    });
  const voiceOptions = nativeVoices(engine).map((entry) => ({
    value: entry.id,
    label:
      entry.numbered !== undefined
        ? t(
            entry.female
              ? "settings.assistant.tts.voiceNumberedFemale"
              : "settings.assistant.tts.voiceNumberedMale",
            { n: entry.numbered },
          )
        : t(
            entry.female
              ? "settings.assistant.tts.voiceNamedFemale"
              : "settings.assistant.tts.voiceNamedMale",
            { name: entry.id },
          ),
  }));

  return (
    <>
      {engine === "kitten" && (
        <SettingContainer
          title={t("settings.assistant.tts.kittenSizeLabel")}
          info={t("settings.assistant.tts.kittenSizeDescription")}
          layout="horizontal"
          grouped={true}
        >
          <Dropdown
            options={KITTEN_SIZES.map(({ size, modelId }) => ({
              value: modelId,
              label: t(`settings.assistant.tts.kittenSizes.${size}`, {
                size: sizeOf(modelId),
              }),
            }))}
            selectedValue={
              KITTEN_SIZES.find((entry) => entry.size === kittenSizeFor(model))!
                .modelId
            }
            onSelect={onModel}
            disabled={disabled}
            className="min-w-[340px]"
          />
        </SettingContainer>
      )}
      <NativeVoicePackRow
        modelId={packId}
        title={t(`settings.assistant.tts.packLabel.${engine}`)}
        description={(size) =>
          t(`settings.assistant.tts.packDescription.${engine}`, { size })
        }
        unsupported={unsupported}
        disabled={disabled}
      />
      <SettingContainer
        title={t("settings.assistant.tts.voiceLabel")}
        layout="horizontal"
        grouped={true}
      >
        <Dropdown
          options={voiceOptions}
          selectedValue={effectiveNativeVoice(engine, voice)}
          onSelect={onVoice}
          disabled={disabled}
          className="min-w-[340px]"
        />
      </SettingContainer>
    </>
  );
};

/**
 * One downloadable native voice (`native_tts.rs`): Kokoro for the processor,
 * a Kitten size, Pocket TTS or Supertonic. It is an ordinary catalog download
 * to the model manager, so the progress, cancel and remove flows are the same
 * ones every model uses; the shared engine library comes down with the first
 * pack and goes when the last pack is removed.
 */
export const NativeVoicePackRow: React.FC<{
  modelId: string;
  title: string;
  /** Receives the download size, e.g. "350 MB". */
  description: (size: string) => string;
  /** The platform has no native engine build. */
  unsupported?: boolean;
  disabled?: boolean;
}> = ({ modelId, title, description, unsupported, disabled }) => {
  const { t } = useTranslation();
  const model = useModelStore((state) =>
    state.models.find((m) => m.id === modelId),
  );
  const downloading = useModelStore(
    (state) => modelId in state.downloadingModels,
  );
  const verifying = useModelStore((state) => modelId in state.verifyingModels);
  const extracting = useModelStore(
    (state) => modelId in state.extractingModels,
  );
  const progress = useModelStore((state) => state.downloadProgress[modelId]);
  const downloadModel = useModelStore((state) => state.downloadModel);
  const cancelDownload = useModelStore((state) => state.cancelDownload);
  const deleteModel = useModelStore((state) => state.deleteModel);

  if (!model) return null;
  const size = t("settings.assistant.tts.packSize", {
    size: model.size_mb,
  });
  const percent = Math.max(
    0,
    Math.min(100, Math.round(progress?.percentage ?? 0)),
  );
  const settingUp = verifying || extracting;

  return (
    <SettingContainer
      title={title}
      description={description(size)}
      descriptionMode="inline"
      layout="horizontal"
      grouped={true}
    >
      <div className="flex min-w-[260px] justify-end">
        {unsupported ? (
          <span className="text-xs text-muted">
            {t("settings.assistant.tts.packUnsupported")}
          </span>
        ) : model.is_downloaded ? (
          <div className="flex items-center gap-2">
            <span className="inline-flex items-center gap-1.5 text-[13px] font-medium text-accent">
              <Check className="h-4 w-4" />
              {t("settings.assistant.tts.kokoroReady")}
            </span>
            <Button
              variant="danger-ghost"
              size="sm"
              onClick={() => void deleteModel(modelId)}
              disabled={disabled}
            >
              {t("settings.assistant.tts.packRemove")}
            </Button>
          </div>
        ) : downloading || settingUp ? (
          <div className="w-full max-w-[240px] space-y-1.5">
            <div className="flex items-center justify-between gap-3 text-xs text-muted">
              <span className="inline-flex items-center gap-1.5">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                {settingUp
                  ? t("settings.assistant.tts.packInstalling")
                  : t("settings.assistant.tts.packDownloading")}
              </span>
              {!settingUp && (
                <span className="inline-flex items-center gap-2">
                  <span className="tabular-nums">{percent}%</span>
                  <button
                    type="button"
                    onClick={() => void cancelDownload(modelId)}
                    className="rounded p-0.5 text-muted hover:bg-ink/[0.06] hover:text-ink"
                    aria-label={t("settings.assistant.tts.packCancel")}
                    title={t("settings.assistant.tts.packCancel")}
                  >
                    <X className="h-3.5 w-3.5" />
                  </button>
                </span>
              )}
            </div>
            <div className="h-1.5 overflow-hidden rounded-full bg-hairline-strong">
              <div
                className="h-full rounded-full bg-accent transition-[width] duration-200"
                style={{ width: `${settingUp ? 100 : percent}%` }}
              />
            </div>
          </div>
        ) : (
          <Button
            variant="primary-soft"
            size="sm"
            onClick={() => void downloadModel(modelId)}
            disabled={disabled}
          >
            <Download className="h-3.5 w-3.5" />
            {t("settings.assistant.tts.packDownload", { size })}
          </Button>
        )}
      </div>
    </SettingContainer>
  );
};
