import { type FC, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { SettingContainer } from "../ui/SettingContainer";
import { Dropdown, type DropdownOption } from "../ui/Dropdown";
import { useSettings } from "../../hooks/useSettings";
import { commands } from "@/bindings";
import type { TranscribeAcceleratorSetting } from "@/bindings";

interface AccelerationSelectorProps {
  descriptionMode?: "tooltip" | "inline";
  grouped?: boolean;
}

const GPU_PREFIX = "gpu:";

/**
 * One dropdown value carries both settings:
 *   "auto"       → accelerator=auto, device=null
 *   "cpu"        → accelerator=cpu,  device=null
 *   "gpu:<key>"  → accelerator=gpu,  device=<stable device key>
 */
function encodeSpeechProcessing(
  accelerator: TranscribeAcceleratorSetting,
  gpuDevice: string | null,
): string {
  if (accelerator === "cpu") return "cpu";
  if (accelerator === "gpu" && gpuDevice !== null) {
    return `${GPU_PREFIX}${gpuDevice}`;
  }
  return "auto";
}

function decodeSpeechProcessing(value: string): {
  accelerator: TranscribeAcceleratorSetting;
  gpuDevice: string | null;
} {
  if (value === "cpu") return { accelerator: "cpu", gpuDevice: null };
  if (value.startsWith(GPU_PREFIX)) {
    return { accelerator: "gpu", gpuDevice: value.slice(GPU_PREFIX.length) };
  }
  return { accelerator: "auto", gpuDevice: null };
}

function formatVram(totalVramMb: number): string {
  return totalVramMb >= 1024
    ? `${(totalVramMb / 1024).toFixed(1)} GB`
    : `${totalVramMb} MB`;
}

/**
 * Where local speech-to-text runs: Auto, one specific graphics card, or the
 * processor. Device keys come from the backend and are stable across launches.
 */
export const AccelerationSelector: FC<AccelerationSelectorProps> = ({
  descriptionMode = "tooltip",
  grouped = false,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();

  const [options, setOptions] = useState<DropdownOption[]>([]);

  useEffect(() => {
    let cancelled = false;
    commands.getAvailableAccelerators().then((available) => {
      if (cancelled) return;
      const opts: DropdownOption[] = [];
      if (available.transcribe.includes("auto")) {
        opts.push({
          value: "auto",
          label: t("settings.advanced.acceleration.auto"),
        });
      }
      if (available.transcribe.includes("gpu")) {
        for (const device of available.gpu_devices) {
          opts.push({
            value: `${GPU_PREFIX}${device.id}`,
            // A device name and a size, not prose: nothing to translate.
            label:
              device.total_vram_mb > 0
                ? `${device.name} (${formatVram(device.total_vram_mb)})`
                : device.name,
          });
        }
      }
      if (available.transcribe.includes("cpu")) {
        opts.push({
          value: "cpu",
          label: t("settings.advanced.acceleration.cpu"),
        });
      }
      setOptions(opts);
    });
    return () => {
      cancelled = true;
    };
  }, [t]);

  const current = encodeSpeechProcessing(
    getSetting("transcribe_accelerator") ?? "auto",
    getSetting("transcribe_gpu_device") ?? null,
  );
  // A saved GPU that is gone (or a choice this machine cannot offer) shows as
  // the first option, which is what the engine falls back to.
  const displayed = options.some((option) => option.value === current)
    ? current
    : (options[0]?.value ?? null);

  const handleChange = async (value: string) => {
    const { accelerator, gpuDevice } = decodeSpeechProcessing(value);
    // The device first: a "gpu" saved without one reads as Auto.
    await updateSetting("transcribe_gpu_device", gpuDevice);
    await updateSetting("transcribe_accelerator", accelerator);
  };

  return (
    <SettingContainer
      title={t("settings.advanced.acceleration.title")}
      description={t("settings.advanced.acceleration.description")}
      descriptionMode={descriptionMode}
      grouped={grouped}
      layout="horizontal"
    >
      <Dropdown
        options={options}
        selectedValue={displayed}
        onSelect={handleChange}
        disabled={
          isUpdating("transcribe_accelerator") ||
          isUpdating("transcribe_gpu_device")
        }
      />
    </SettingContainer>
  );
};
