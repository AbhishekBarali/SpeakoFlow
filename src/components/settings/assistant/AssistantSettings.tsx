import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { toast } from "sonner";
import {
  ArrowUpRight,
  Check,
  Loader2,
  AudioLines,
  Volume2,
  Download,
  Globe,
  Keyboard,
  Sparkles,
  PanelTop,
  Power,
  PlugZap,
} from "lucide-react";
import {
  commands,
  type TtsVoice,
  type Result,
  type LocalLlmStatus,
  type AssistantResponseLength,
  type AssistantSearchDepth,
  type AskAnchor,
  type AudioTagIntensity,
  type DisplayChoice,
  type ModelChoice,
  type ModelUnloadTimeout,
} from "@/bindings";
import { ElevenLabsExpressiveness } from "./ElevenLabsExpressiveness";
import {
  Dropdown,
  SettingContainer,
  SettingsGroup,
  Slider,
  ToggleSwitch,
} from "@/components/ui";
import { InfoTip } from "@/components/ui/InfoTip";
import { Input } from "../../ui/Input";
import { ModelCombo } from "../../ui/ModelCombo";
import { LogoChoice } from "../../ui/LogoChoice";
import { ProviderTile } from "../../icons/ProviderLogos";
import { Button } from "../../ui/Button";
import { LlmModelPicker } from "@/components/shell/ModelPicker";
import { ProviderModeToggle } from "../PostProcessingSettingsApi/ProviderModeToggle";
import { ProviderSelect } from "../PostProcessingSettingsApi/ProviderSelect";
import { ShortcutInput } from "../ShortcutInput";
import { PushToTalk } from "../PushToTalk";
import { RemindersSettings } from "./RemindersSettings";
import { ThinkingLevelSetting } from "../ThinkingLevelSetting";
import { useSettings } from "../../../hooks/useSettings";
import { useKokoroTts } from "../../../assistant/useKokoroTts";
import { localTtsActive } from "../../../assistant/localTts";
import {
  KOKORO_NATIVE_MODEL_ID,
  parseKokoroDevice,
  useLocalVoiceStatus,
} from "../../../assistant/localVoice";
import { NativeEngineRows, NativeVoicePackRow } from "./NativeVoicePack";
import { useModelStore } from "@/stores/modelStore";
import {
  TTS_ENGINES,
  elevenLabsModelSupportsAudioTags,
  hostOf,
  ttsEngineSpec,
  ttsNeedsSetup,
  ttsValues,
} from "@/lib/ttsEngines";
import { isNativeEngine, nativePackId } from "@/lib/nativeVoices";
import { FONT_SIZES } from "../../../assistant/appearance";
import QuickAsk from "../../../assistant/QuickAsk";
import {
  askDisplayOptions,
  askDisplayValue,
} from "@/components/pages/assistant/panelGeometry";
import "../../../assistant/AssistantPanel.css";
import { useLocalLlmEngineStatus } from "@/hooks/useLocalLlmEngineStatus";

/** The built-in (local) llama.cpp provider id, mirrored from the backend. */
const BUILTIN_PROVIDER_ID = "builtin";

const KOKORO_DTYPES = ["fp32", "fp16", "q8", "q4", "q4f16"] as const;

/** Voice names stay as-is; only the accent/gender descriptor is translated. */
const KOKORO_VOICES = [
  { value: "af_heart", name: "Heart", kind: "usFemale" },
  { value: "af_bella", name: "Bella", kind: "usFemale" },
  { value: "af_nicole", name: "Nicole", kind: "usFemaleSoft" },
  { value: "af_sky", name: "Sky", kind: "usFemale" },
  { value: "am_adam", name: "Adam", kind: "usMale" },
  { value: "am_michael", name: "Michael", kind: "usMale" },
  { value: "bf_emma", name: "Emma", kind: "ukFemale" },
  { value: "bm_george", name: "George", kind: "ukMale" },
] as const;

/** Quick-pick playback speeds for the TTS speed control. Users can also type
 *  an arbitrary value (clamped to 0.25–4 by the backend). */
const TTS_SPEED_PRESETS = [0.5, 1, 1.5, 2, 3];

/** Rotating set of playful lines spoken by the "Test voice" button. One is
 *  picked at random on each press instead of always saying the same thing.
 *  These are intentionally not translated — they're meme sample lines. */
const TEST_PHRASES = [
  "Wagwan brother.",
  "Hi! This is a test of SpeakoFlow's voice output.",
  "Ayo, is this thing on?",
  "Greetings, human. Your voice assistant has entered the chat.",
  "Testing, testing, one two... yeah we good.",
  "Beep boop, I am definitely not a robot.",
  "Loud and clear, captain.",
  "Yo, mic check. Sounding crispy.",
];

/** Sample lines for an ElevenLabs voice with audio tags on, so the test
 *  plays what the feature actually does: a performance, with reactions and
 *  sound effects, rather than a plain read. Not translated, like the above;
 *  the tags themselves are English whatever the language. */
const AUDIO_TAG_TEST_PHRASES = [
  "[excited] Audio tags are on! [laughs] Now I can actually perform, not just talk.",
  "[whispers] Can you keep a secret? [normal voice] I can whisper now. [chuckles]",
  "[crowd cheering] [applause] Thank you, thank you! [laughs] You're all too kind.",
  "[drumroll] And the award for best voice assistant goes to... [gasps] me! [applause]",
  "[sighs] Another Monday. [mischievously] But hey, at least I sound great.",
];

/** Pick a random sample line for the voice test. */
const randomTestPhrase = (audioTags = false): string => {
  const phrases = audioTags ? AUDIO_TAG_TEST_PHRASES : TEST_PHRASES;
  return phrases[Math.floor(Math.random() * phrases.length)];
};

/** Audio-tag intensities in slider order, left (least) to right (most). */
const AUDIO_TAG_INTENSITIES: readonly AudioTagIntensity[] = [
  "subtle",
  "balanced",
  "theatrical",
];

/** Editable model/voice picker (input + datalist + refresh). Shared with the
 *  dictation-cleanup model field via `@/components/ui/ModelCombo` so the two
 *  never drift. Aliased here to keep the existing call sites unchanged. */
const LoadableSelect = ModelCombo;

/** Live preview of the quick ask. Renders the REAL surface component with
 *  sample content, from the panel's own stylesheet, so the preview and the
 *  panel cannot drift. */
export const PanelPreview: React.FC<{
  fontSize: string;
  opacity: number;
}> = ({ fontSize, opacity }) => {
  const { t } = useTranslation();
  const fs = FONT_SIZES[fontSize] ?? FONT_SIZES.medium;
  const inert = {
    input: "",
    onInputChange: noop,
    onSubmit: noop,
    onClose: noop,
    onCancel: noop,
    onStop: noop,
    onRetry: noop,
    onInsert: async () => false,
    notice: null,
    error: null,
    screen: false,
  };

  return (
    <div
      className="assistant-scope assistant-preview"
      aria-hidden="true"
      style={
        {
          "--as-msg-font": fs,
          "--as-alpha": String(Math.max(opacity, 0.5)),
        } as React.CSSProperties
      }
    >
      {/* The two shapes the surface takes, in the order they happen: the pill a
          question opens in, and the card the answer unfolds into. */}
      <div className="assistant-preview-stack">
        <QuickAsk
          {...inert}
          phase="listening"
          status={t("assistant.status.listening")}
          levels={PREVIEW_LEVELS}
          question=""
          answer=""
          selectionChars={0}
          canRetry={false}
        />
        <QuickAsk
          {...inert}
          phase="done"
          status=""
          question={t("settings.assistant.appearance.previewUser")}
          answer={t("settings.assistant.appearance.previewAssistant")}
          selectionChars={0}
          canRetry={false}
        />
      </div>
    </div>
  );
};

const noop = () => {};
const PREVIEW_LEVELS = [0.3, 0.6, 0.4, 0.8, 0.5, 0.3, 0.7, 0.4, 0.55, 0.3];

/** A block of the assistant's settings. Pages render the ones they own: the
 *  Models page shows `brain` and `voice`, the Assistant page opens each of the
 *  rest in its own dialog. */
export type AssistantSettingsSection =
  | "master"
  | "shortcuts"
  | "brain"
  | "voice"
  | "webSearch"
  | "reminders"
  | "appearance"
  | "behavior";

interface AssistantSettingsProps {
  /** Open the on-device model catalog (owned by the parent page). */
  onOpenLlmCatalog?: () => void;
  /** Only render these sections. Omit for the full page. */
  sections?: AssistantSettingsSection[];
}

export const AssistantSettings: React.FC<AssistantSettingsProps> = ({
  onOpenLlmCatalog,
  sections,
}) => {
  const { t } = useTranslation();
  const show = (section: AssistantSettingsSection) =>
    !sections || sections.includes(section);
  // A single section is shown under its own page or dialog heading, so the
  // group's title would only repeat it.
  const groupTitle = (text: string) =>
    sections && sections.length === 1 ? undefined : text;
  const {
    settings,
    refreshSettings,
    updatePostProcessApiKey,
    updatePostProcessBaseUrl,
  } = useSettings();

  const providers = settings?.post_process_providers || [];
  const selectedProviderId = settings?.assistant_provider_id || "custom";
  const selectedProvider = providers.find((p) => p.id === selectedProviderId);

  // Built-in (local) provider: model is chosen from downloaded LLM models and
  // there is no API key. The engine is the bundled llama.cpp sidecar.
  const isBuiltin = selectedProviderId === BUILTIN_PROVIDER_ID;

  const [localLlmStatus, setLocalLlmStatus] = useState<LocalLlmStatus | null>(
    null,
  );
  // Live built-in engine (llama.cpp binary) setup progress, shared with the
  // assistant panel via the same backend events, so this form can show the
  // one-time first-run engine download instead of leaving it invisible.
  const engineStatus = useLocalLlmEngineStatus();
  useEffect(() => {
    if (!isBuiltin) return;
    let active = true;
    void commands.getLocalLlmStatus().then((res) => {
      if (active && res.status === "ok") setLocalLlmStatus(res.data);
    });
    return () => {
      active = false;
    };
  }, [isBuiltin]);

  const [model, setModel] = useState("");
  const [historyLimit, setHistoryLimit] = useState("12");
  const [contextSize, setContextSize] = useState("8192");

  /**
   * The displays currently attached, for the "Which screen" row.
   *
   * Fetched rather than derived: only the backend can enumerate monitors, and the
   * list changes while the app runs. Re-read whenever the window regains focus,
   * because plugging a screen in is exactly the kind of thing someone does and then
   * comes straight back to this panel to point the assistant at it.
   */
  const [displays, setDisplays] = useState<DisplayChoice[]>([]);
  useEffect(() => {
    let active = true;
    const load = () => {
      void commands.listAssistantDisplays().then((res) => {
        if (active && res.status === "ok") setDisplays(res.data);
      });
    };
    load();
    window.addEventListener("focus", load);
    return () => {
      active = false;
      window.removeEventListener("focus", load);
    };
  }, []);

  const [apiKey, setApiKey] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  // Which credential field currently has focus, so the settings→state resync
  // effect can leave an in-progress edit alone. A ref, not state: it must be
  // readable by that effect without itself triggering a render.
  const editingCredentialField = useRef<"apiKey" | "baseUrl" | null>(null);
  const [ttsBaseUrl, setTtsBaseUrl] = useState("");
  const [ttsApiKey, setTtsApiKey] = useState("");
  const [ttsModel, setTtsModel] = useState("");
  const [ttsRemoteVoice, setTtsRemoteVoice] = useState("");
  // Manual playback-speed entry (string while editing; committed on blur).
  const [ttsSpeedInput, setTtsSpeedInput] = useState("1");

  // Which cloud provider to return to when the brain picker flips back from "On
  // my device". Read from settings, not component state: `assistant_provider_id`
  // is a single slot that the device switch overwrites, and keeping the memory
  // in React meant it survived a toggle but not a navigation away and back —
  // the user returned, chose Cloud, and landed on someone else's provider with
  // their model apparently gone.
  const lastCloudProviderId =
    settings?.assistant_last_cloud_provider_id ?? null;
  const providerSwitchSequence = useRef(0);
  const providerSwitchQueue = useRef<Promise<void>>(Promise.resolve());
  const [isProviderSwitching, setIsProviderSwitching] = useState(false);

  // Brain connection test state. Kept separate from the TTS and web-search test
  // states so one failing test can't blank another's message.
  const [brainTest, setBrainTest] = useState<
    "idle" | "testing" | "ok" | "error"
  >("idle");
  const [brainTestMsg, setBrainTestMsg] = useState<string | null>(null);

  // Web search section state.
  const [webSearchApiKey, setWebSearchApiKey] = useState("");
  const [webSearchTest, setWebSearchTest] = useState<
    "idle" | "testing" | "ok" | "error"
  >("idle");
  const [webSearchTestMsg, setWebSearchTestMsg] = useState<string | null>(null);

  // TTS test button state (shared across engines).
  const [testState, setTestState] = useState<
    "idle" | "testing" | "ok" | "error"
  >("idle");
  const [testError, setTestError] = useState<string | null>(null);

  // Assistant model list, fetched per-provider from its /models endpoint via
  // the same command post-processing uses (providers + keys are shared). Kept
  // local to this component so it doesn't couple to the post-processing tab.
  const [loadedModels, setLoadedModels] = useState<
    Record<string, ModelChoice[]>
  >({});
  const [modelsLoading, setModelsLoading] = useState(false);
  const [modelsError, setModelsError] = useState<string | null>(null);

  // Remote TTS voice / model lists, loaded on demand for the OpenAI-compatible,
  // ElevenLabs, and Azure engines (searchable pickers instead of raw text
  // fields).
  const [ttsVoiceList, setTtsVoiceList] = useState<TtsVoice[]>([]);
  const [ttsVoicesLoading, setTtsVoicesLoading] = useState(false);
  const [ttsVoicesError, setTtsVoicesError] = useState<string | null>(null);
  const [ttsModelList, setTtsModelList] = useState<string[]>([]);
  const [ttsModelsLoading, setTtsModelsLoading] = useState(false);
  const [ttsModelsError, setTtsModelsError] = useState<string | null>(null);

  const ttsEngine = settings?.assistant_tts_engine ?? "kokoro";
  const ttsEnabled = settings?.assistant_tts_enabled ?? false;
  const ttsVoice = settings?.assistant_tts_voice ?? "af_heart";
  const ttsDtype = settings?.assistant_tts_kokoro_dtype ?? "fp32";
  const ttsSpeed = settings?.assistant_tts_speed ?? 1;
  const ttsSpec = ttsEngineSpec(ttsEngine);
  const ttsEngineName = t(`voiceEngines.names.${ttsEngine}`, {
    defaultValue: ttsEngine,
  });
  const ttsHasSpeed = ttsSpec?.speed != null;
  // ElevenLabs audio tags. The model check reads the field as shown, so the
  // switch reacts as soon as a model is picked; the backend applies the same
  // rule (`audio_tags::active`) to the model it actually requests.
  const audioTagsOn = settings?.assistant_tts_elevenlabs_audio_tags ?? false;
  const audioTagsModelOk = elevenLabsModelSupportsAudioTags(ttsModel);
  const audioTagsActive =
    ttsEngine === "elevenlabs" && audioTagsOn && audioTagsModelOk;
  const audioTagIntensity: AudioTagIntensity =
    settings?.assistant_tts_elevenlabs_audio_tag_intensity ?? "balanced";
  // What speed does on this engine: its own range, or that it has none.
  const ttsSpeedHelp = [
    t("settings.assistant.tts.speedDescription"),
    ttsSpec?.speed
      ? t("settings.assistant.tts.speedRange", {
          engine: ttsEngineName,
          min: ttsSpec.speed[0],
          max: ttsSpec.speed[1],
        })
      : t("settings.assistant.tts.speedUnsupported", {
          engine: ttsEngineName,
        }),
  ].join(" ");
  // The voice row's help: engines with a well-known id format say what it
  // looks like; the rest point at Load voices.
  const ttsVoiceHelp =
    ttsEngine === "elevenlabs"
      ? t("settings.assistant.tts.elevenVoiceDescription")
      : ttsEngine === "azure"
        ? t("settings.assistant.tts.azureVoiceDescription")
        : ttsEngine === "deepgram"
          ? t("settings.assistant.tts.deepgramVoiceDescription")
          : ttsEngine === "openai" || ttsEngine === "custom"
            ? t("settings.assistant.tts.remoteVoiceDescription")
            : t("settings.assistant.tts.voiceLoadDescription");

  // TTS fields save on blur. Serialize those saves with actions such as Load
  // and Test so a click can never overtake the blur that contains a newly typed
  // API key/model/voice (the previous race sent OpenRouter an unauthenticated
  // request even while the password field visibly contained a key).
  const ttsTaskQueue = useRef<Promise<void>>(Promise.resolve());
  const queueTtsTask = (task: () => Promise<void>): Promise<void> => {
    const next = ttsTaskQueue.current.catch(() => undefined).then(task);
    ttsTaskQueue.current = next;
    return next;
  };

  const runTtsCommand = async (command: Promise<Result<null, string>>) => {
    const result = await command;
    if (result.status === "error") throw new Error(result.error);
  };

  /** Persist every visible remote-TTS draft before an action consumes it. */
  const persistRemoteTtsDraft = async () => {
    let changed = false;
    const spec = ttsEngineSpec(ttsEngine);
    const remote = !!spec && !spec.local;
    if (spec?.url && ttsBaseUrl !== (settings?.assistant_tts_base_url ?? "")) {
      await runTtsCommand(commands.setAssistantTtsBaseUrl(ttsBaseUrl));
      changed = true;
    }
    if (
      remote &&
      spec.key !== "none" &&
      ttsApiKey !== (settings?.assistant_tts_api_key ?? "")
    ) {
      await runTtsCommand(commands.setAssistantTtsApiKey(ttsApiKey));
      changed = true;
    }
    if (
      spec?.model &&
      ttsModel.trim() !== (settings?.assistant_tts_model ?? "").trim()
    ) {
      await runTtsCommand(commands.setAssistantTtsModel(ttsModel.trim()));
      changed = true;
    }
    if (
      remote &&
      ttsRemoteVoice.trim() !==
        (settings?.assistant_tts_remote_voice ?? "").trim()
    ) {
      await runTtsCommand(
        commands.setAssistantTtsRemoteVoice(ttsRemoteVoice.trim()),
      );
      changed = true;
    }

    const parsedSpeed = Number.parseFloat(ttsSpeedInput);
    if (Number.isFinite(parsedSpeed)) {
      const clampedSpeed = Math.min(4, Math.max(0.25, parsedSpeed));
      if (clampedSpeed !== ttsSpeed) {
        await runTtsCommand(commands.setAssistantTtsSpeed(clampedSpeed));
        changed = true;
      }
    }
    if (changed) await refreshSettings();
  };

  // Fetch the model list for the selected assistant provider from its /models
  // endpoint (shares the post-processing command since providers + keys are
  // shared). Errors surface inline; the field stays a free-text search so a
  // provider without a model list is never a dead end.
  const handleLoadAssistantModels = async () => {
    setModelsLoading(true);
    setModelsError(null);
    try {
      const res = await commands.fetchPostProcessModels(selectedProviderId);
      if (res.status === "error") {
        setModelsError(res.error);
        return;
      }
      setLoadedModels((prev) => ({ ...prev, [selectedProviderId]: res.data }));
      if (res.data.length === 0) {
        setModelsError(t("settings.assistant.provider.noModelsFound"));
      }
    } catch (e) {
      setModelsError(String(e));
    } finally {
      setModelsLoading(false);
    }
  };

  const handleAssistantModelChange = async (value: string) => {
    const trimmed = value.trim();
    if (!trimmed) return;
    setModel(trimmed);
    await commands.changeAssistantModelSetting(selectedProviderId, trimmed);
    await refreshSettings();
  };

  // Load selectable voices / models for the current remote TTS engine.
  // Await pending field saves first so discovery uses the value on screen.
  const handleLoadTtsVoices = async () => {
    setTtsVoicesLoading(true);
    setTtsVoicesError(null);
    try {
      await queueTtsTask(persistRemoteTtsDraft);
      const res = await commands.assistantListTtsVoices();
      if (res.status === "error") {
        setTtsVoicesError(res.error);
        setTtsVoiceList([]);
        return;
      }
      setTtsVoiceList(res.data);
    } catch (e) {
      setTtsVoicesError(String(e));
      setTtsVoiceList([]);
    } finally {
      setTtsVoicesLoading(false);
    }
  };

  const handleLoadTtsModels = async () => {
    setTtsModelsLoading(true);
    setTtsModelsError(null);
    try {
      await queueTtsTask(persistRemoteTtsDraft);
      const res = await commands.assistantListTtsModels();
      if (res.status === "error") {
        setTtsModelsError(res.error);
        setTtsModelList([]);
        return;
      }
      setTtsModelList(res.data);
    } catch (e) {
      setTtsModelsError(String(e));
      setTtsModelList([]);
    } finally {
      setTtsModelsLoading(false);
    }
  };
  /** Master switch. Default on, matching the backend's `default_true`. */
  const assistantEnabled = settings?.assistant_enabled ?? true;
  const kokoroDevice = parseKokoroDevice(settings?.assistant_tts_kokoro_device);
  // Where the local voice actually speaks is Rust's decision (the graphics card
  // in the panel, or the processor engine). Probing WebGPU here costs no model
  // load and lets Automatic say where it will speak before any call.
  const localVoiceShown =
    assistantEnabled &&
    ttsEnabled &&
    (ttsEngine === "kokoro" || isNativeEngine(ttsEngine));
  const localVoice = useLocalVoiceStatus({
    enabled: localVoiceShown,
    probe: localVoiceShown && ttsEngine === "kokoro" && kokoroDevice === "auto",
  });
  const kokoroOnProcessor =
    ttsEngine === "kokoro" && localVoice?.route === "native";
  const storeModels = useModelStore((state) => state.models);
  /** Whether a native engine's pack (for Kitten, the size it is set to) is on
   *  disk, which is all that engine needs to speak. */
  const nativeEngineReady = (engine: string): boolean => {
    const packId = nativePackId(engine, ttsValues(settings, engine).model);
    return (
      !!packId && !!storeModels.find((m) => m.id === packId)?.is_downloaded
    );
  };
  // The processor voice is a 350 MB download, so it is offered only where it
  // helps: Kokoro set to the processor, a graphics card that can't run it, or a
  // pack already on disk (or on its way) that may need removing.
  const kokoroNativePresent = useModelStore((state) => {
    const id = KOKORO_NATIVE_MODEL_ID;
    return (
      !!state.models.find((m) => m.id === id)?.is_downloaded ||
      id in state.downloadingModels ||
      id in state.verifyingModels ||
      id in state.extractingModels
    );
  });
  const offerProcessorVoice =
    kokoroDevice === "cpu" ||
    localVoice?.webgpu === "unusable" ||
    kokoroNativePresent;
  // One plain sentence for where Kokoro is speaking right now, and why.
  const runsOnStatus = (() => {
    const status = "settings.assistant.tts.runsOnStatus";
    if (!localVoice) return t(`${status}.checking`);
    if (localVoice.route === "native") {
      return kokoroDevice === "auto"
        ? t(`${status}.gpuUnusable`)
        : t(`${status}.processor`);
    }
    if (kokoroDevice === "gpu") {
      return localVoice.webgpu === "unusable"
        ? t(`${status}.gpuForcedUnusable`)
        : t(`${status}.gpu`);
    }
    // Downloaded but refused to start (e.g. macOS library validation): don't
    // ask for a download that is already there.
    if (localVoice.native_load_failed) return t(`${status}.loadFailed`);
    if (kokoroDevice === "cpu") {
      return localVoice.native_supported
        ? t(`${status}.cpuNeedsPack`)
        : t(`${status}.processor`);
    }
    if (localVoice.webgpu === "usable") return t(`${status}.gpu`);
    if (localVoice.webgpu === "unusable") {
      return localVoice.native_supported
        ? t(`${status}.needsPack`)
        : t(`${status}.noNativeSlow`);
    }
    return t(`${status}.checking`);
  })();
  // Kokoro on a small processor is slow even natively (4 logical cores run it
  // on 2 threads). Offer the lighter voice there, only when the processor is
  // what speaks, and never when the graphics card works.
  const smallProcessor =
    typeof navigator !== "undefined" &&
    (navigator.hardwareConcurrency ?? 8) <= 4;
  const suggestKitten =
    ttsEngine === "kokoro" &&
    smallProcessor &&
    !!localVoice?.native_supported &&
    (kokoroDevice === "cpu" ||
      (kokoroDevice === "auto" && localVoice.webgpu === "unusable"));
  // Local speech only makes sense when the assistant is on AND the local engine
  // is the selected one — otherwise this page must not hold the weights at all.
  // Nor when Kokoro is routed to the processor engine, which is not this page.
  const kokoroEnabled =
    assistantEnabled &&
    localTtsActive(ttsEnabled, ttsEngine) &&
    !kokoroOnProcessor;
  // Settings must never download Kokoro just because this page mounted. The
  // live assistant still loads on an actual spoken reply; here, only Setup or
  // Test voice may call prepare/speak.
  const kokoroTest = useKokoroTts(
    kokoroEnabled,
    ttsVoice,
    ttsDtype,
    ttsSpeed,
    false,
    undefined,
    kokoroDevice,
  );
  const {
    status: kokoroStatus,
    progress: kokoroProgress,
    error: kokoroError,
  } = kokoroTest;
  // Kokoro is downloading (or loading) inside this page. Test voice waits for
  // it: the row above already shows the progress, and a test pressed mid-way
  // could only queue behind the same download.
  const kokoroDownloading =
    ttsEngine === "kokoro" && !kokoroOnProcessor && kokoroStatus === "loading";
  // "q8-cpu" is the q8 graph with WebGPU disabled, so it downloads and caches
  // exactly the same weights file. Strip the suffix before anything that keys
  // off the precision, or switching to it would re-prompt a download that has
  // already happened.
  const ttsDtypeBase = ttsDtype.endsWith("-cpu")
    ? ttsDtype.slice(0, -"-cpu".length)
    : ttsDtype;
  const kokoroReadyKey = `speakoflow.kokoro.ready.${ttsDtypeBase}`;
  /** Whether Kokoro's weights are already on this device; `null` until the
   *  cache has been checked, so the row doesn't flash a Download button for
   *  the few milliseconds the check takes. */
  const [kokoroPrepared, setKokoroPrepared] = useState<boolean | null>(null);

  // Reflect the ACTUAL browser cache, not just a local flag. kokoro-js
  // (transformers.js) caches model weights in the "transformers-cache" Cache
  // Storage; if this precision's weights are already present, the model is
  // ready and we must NOT prompt a re-download. This fixes "have to click
  // Download every time you switch": the old flag was keyed per-precision and
  // only set from this page, so it desynced from reality (e.g. after the live
  // panel had already downloaded the model). Falls back to the local flag if
  // the Cache API is unavailable, so there's no regression.
  useEffect(() => {
    let cancelled = false;
    const dtypeSuffix: Record<string, string> = {
      fp32: "",
      fp16: "_fp16",
      q8: "_quantized",
      int8: "_int8",
      uint8: "_uint8",
      q4: "_q4",
      q4f16: "_q4f16",
      bnb4: "_bnb4",
    };
    const refresh = async () => {
      // The cache is the truth wherever it can be read. The flag is only a
      // fallback for a WebView without the Cache API: trusted over the cache,
      // a flag left behind after the cache was evicted would say "Ready" while
      // a Test voice quietly downloaded 300 MB.
      let prepared: boolean;
      try {
        if (typeof caches !== "undefined") {
          const cache = await caches.open("transformers-cache");
          const urls = (await cache.keys())
            .map((r) => r.url)
            .filter((u) => u.includes("Kokoro-82M"));
          const file = `model${dtypeSuffix[ttsDtypeBase] ?? ""}.onnx`;
          prepared =
            urls.some((u) => u.includes(file)) ||
            urls.some((u) => u.endsWith(".onnx"));
        } else {
          prepared = window.localStorage.getItem(kokoroReadyKey) === "true";
        }
      } catch {
        prepared = window.localStorage.getItem(kokoroReadyKey) === "true";
      }
      if (!cancelled) setKokoroPrepared(prepared);
    };
    void refresh();
    return () => {
      cancelled = true;
    };
  }, [kokoroReadyKey, ttsDtypeBase]);

  const rememberKokoroReady = () => {
    window.localStorage.setItem(kokoroReadyKey, "true");
    setKokoroPrepared(true);
  };

  const handlePrepareKokoro = async () => {
    try {
      await kokoroTest.prepare();
      rememberKokoroReady();
    } catch {
      // The hook exposes a precise error state in the setup row.
    }
  };

  const handleTestTts = async () => {
    setTestState("testing");
    setTestError(null);
    const phrase = randomTestPhrase(audioTagsActive);
    try {
      if (ttsEngine === "kokoro" && !kokoroOnProcessor) {
        await kokoroTest.prepare();
        rememberKokoroReady();
        const outcome = await kokoroTest.speak(phrase, true);
        if (outcome === "gpu" || outcome === "muted" || outcome === "failed") {
          setTestState("error");
          setTestError(
            outcome === "failed"
              ? t("settings.assistant.tts.testFailed")
              : outcome === "muted"
                ? t("settings.assistant.tts.testGpuSuspect")
                : kokoroDevice === "gpu"
                  ? t("settings.assistant.tts.testGpuForced")
                  : t("settings.assistant.tts.testGpuMoved"),
          );
          return;
        }
      } else {
        await queueTtsTask(persistRemoteTtsDraft);
        const res = await commands.assistantTestTts(phrase);
        if (res.status === "error") {
          setTestState("error");
          setTestError(res.error);
          return;
        }
      }
      setTestState("ok");
      setTimeout(() => setTestState("idle"), 2000);
    } catch (e) {
      setTestState("error");
      setTestError(String(e));
    }
  };

  useEffect(() => {
    setModel(settings?.assistant_models?.[selectedProviderId] ?? "");
    // Never overwrite a credential field while the user is typing in it. This
    // effect depends on the WHOLE `settings` object, so any settings write
    // anywhere in the app re-runs it — a finished dictation, the 30-minute
    // retention sweep, or a neighbouring control calling `refreshSettings`.
    // Because the key is persisted on blur, clobbering a half-entered value
    // also wrote the OLD key straight back on the way out: a freshly pasted key
    // silently reverted, but only when an unrelated event happened to land in
    // the gap between the paste and the blur, which is what made it look
    // random. A provider switch blurs the input first, so the resync a switch
    // needs still happens.
    if (editingCredentialField.current !== "apiKey") {
      setApiKey(settings?.post_process_api_keys?.[selectedProviderId] ?? "");
    }
    if (editingCredentialField.current !== "baseUrl") {
      setBaseUrl(selectedProvider?.base_url ?? "");
    }
  }, [settings, selectedProviderId, selectedProvider]);

  useEffect(() => {
    setHistoryLimit(String(settings?.assistant_max_history_messages ?? 12));
  }, [settings?.assistant_max_history_messages]);

  const handleHistoryLimitBlur = async () => {
    const parsed = Math.max(0, Math.min(200, parseInt(historyLimit, 10) || 0));
    setHistoryLimit(String(parsed));
    await commands.setAssistantMaxHistoryMessages(parsed);
    await refreshSettings();
  };

  // Built-in local engine context window. Mirrors the history-limit pattern:
  // local input state, clamped + persisted on blur. Only shown for the
  // built-in provider (external providers manage their own context).
  useEffect(() => {
    setContextSize(String(settings?.local_llm_context_size ?? 8192));
  }, [settings?.local_llm_context_size]);

  const handleContextSizeBlur = async () => {
    const parsed = Math.max(
      512,
      Math.min(32768, parseInt(contextSize, 10) || 8192),
    );
    setContextSize(String(parsed));
    await commands.setLocalLlmContextSize(parsed);
    await refreshSettings();
  };

  // Idle-unload timeout for the built-in local LLM engine: after this long with
  // no use, the model is unloaded from RAM/VRAM (it reloads on next use). Mirrors
  // the STT model-unload control; the extra 15s option only shows in debug mode.
  const llmUnloadOptions = useMemo(() => {
    // NOTE: the values are the serde snake_case forms the backend actually
    // accepts ("min2", not the "min_2" specta emits for the TS type), matching
    // ModelUnloadTimeout.tsx — hence the casts. Sending the specta form would
    // fail to deserialize and silently not save.
    const base: { value: ModelUnloadTimeout; label: string }[] = [
      {
        value: "never" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.never"),
      },
      {
        value: "immediately" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.immediately"),
      },
      {
        value: "min2" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.min2"),
      },
      {
        value: "min5" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.min5"),
      },
      {
        value: "min10" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.min10"),
      },
      {
        value: "min15" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.min15"),
      },
      {
        value: "hour1" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.hour1"),
      },
    ];
    if (settings?.debug_mode) {
      base.push({
        value: "sec15" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.sec15"),
      });
    }
    return base;
  }, [t, settings?.debug_mode]);

  useEffect(() => {
    setTtsBaseUrl(settings?.assistant_tts_base_url ?? "");
    setTtsApiKey(settings?.assistant_tts_api_key ?? "");
    setTtsModel(settings?.assistant_tts_model ?? "");
    setTtsRemoteVoice(settings?.assistant_tts_remote_voice ?? "");
    setTtsSpeedInput(String(settings?.assistant_tts_speed ?? 1));
  }, [
    settings?.assistant_tts_base_url,
    settings?.assistant_tts_api_key,
    settings?.assistant_tts_model,
    settings?.assistant_tts_remote_voice,
    settings?.assistant_tts_speed,
  ]);

  // Clear any loaded voice/model lists when the TTS engine changes so a stale
  // list (e.g. OpenAI voices) never shows under a different engine.
  useEffect(() => {
    setTtsVoiceList([]);
    setTtsVoicesError(null);
    setTtsModelList([]);
    setTtsModelsError(null);
  }, [settings?.assistant_tts_engine]);

  const providerOptions = useMemo(
    () =>
      providers
        .filter((provider) => provider.id !== "apple_intelligence")
        .map((provider) => ({ value: provider.id, label: provider.label }))
        // Keep the built-in local model pinned to the top — it's the zero-setup,
        // no-API-key option most users should reach for first.
        .sort((a, b) =>
          a.value === BUILTIN_PROVIDER_ID
            ? -1
            : b.value === BUILTIN_PROVIDER_ID
              ? 1
              : 0,
        ),
    [providers],
  );

  // Cloud (non-built-in) providers only, for the "Cloud provider" brain mode.
  const cloudProviderOptions = useMemo(
    () => providerOptions.filter((p) => p.value !== BUILTIN_PROVIDER_ID),
    [providerOptions],
  );

  // Options for the searchable assistant-model picker: loaded models for the
  // current provider plus the currently-set model (so a hand-typed value still
  // shows as selected). The Select is creatable, so users can type any model.
  const assistantModelOptions = useMemo(() => {
    const seen = new Set<string>();
    const opts: { value: string; label: string }[] = [];
    const add = (v?: string | null, label?: string) => {
      const trimmed = v?.trim();
      if (!trimmed || seen.has(trimmed)) return;
      seen.add(trimmed);
      opts.push({ value: trimmed, label: label?.trim() || trimmed });
    };
    for (const m of loadedModels[selectedProviderId] || []) add(m.id, m.label);
    add(model);
    return opts;
  }, [loadedModels, selectedProviderId, model]);

  const ttsVoiceOptions = useMemo(() => {
    const seen = new Set<string>();
    const opts: { value: string; label: string }[] = [];
    const add = (value: string, label: string) => {
      const v = value.trim();
      if (!v || seen.has(v)) return;
      seen.add(v);
      opts.push({ value: v, label: label || v });
    };
    for (const v of ttsVoiceList) add(v.id, v.label);
    if (ttsRemoteVoice.trim()) add(ttsRemoteVoice, ttsRemoteVoice);
    return opts;
  }, [ttsVoiceList, ttsRemoteVoice]);

  const ttsModelOptions = useMemo(() => {
    const seen = new Set<string>();
    const opts: { value: string; label: string }[] = [];
    const add = (v?: string | null) => {
      const trimmed = v?.trim();
      if (!trimmed || seen.has(trimmed)) return;
      seen.add(trimmed);
      opts.push({ value: trimmed, label: trimmed });
    };
    for (const m of ttsModelList) add(m);
    if (ttsModel.trim()) add(ttsModel);
    return opts;
  }, [ttsModelList, ttsModel]);

  // The voice the backend falls back to when the field is left empty, which is
  // exactly what a placeholder should show. Gemini TTS behind OpenRouter takes
  // Google's named voices whatever the engine default is.
  const ttsVoicePlaceholder =
    ttsModel.toLowerCase().includes("gemini") &&
    ttsModel.toLowerCase().includes("tts")
      ? "Kore"
      : ttsSpec?.voice.example || t("pickers.voiceSetup.voicePlaceholder");

  const showProviderSwitchError = () => {
    toast.error(
      t("settings.assistant.provider.saveFailed", {
        defaultValue: "Couldn’t switch the Assistant provider.",
      }),
    );
  };

  const handleProviderSelect = (providerId: string) => {
    const isValidTarget =
      providerId === BUILTIN_PROVIDER_ID ||
      cloudProviderOptions.some((option) => option.value === providerId);
    if (!isValidTarget) {
      showProviderSwitchError();
      return;
    }

    const sequence = ++providerSwitchSequence.current;
    setIsProviderSwitching(true);
    const run = providerSwitchQueue.current
      .catch(() => undefined)
      .then(async () => {
        // Skip a queued choice that was superseded before its write began.
        if (sequence !== providerSwitchSequence.current) return;
        try {
          const result = await commands.setAssistantProvider(providerId);
          if (sequence !== providerSwitchSequence.current) return;
          if (result.status !== "ok") {
            showProviderSwitchError();
            return;
          }
          await refreshSettings();
        } catch (error) {
          console.error("Failed to switch Assistant provider:", error);
          if (sequence === providerSwitchSequence.current) {
            showProviderSwitchError();
          }
        }
      });
    providerSwitchQueue.current = run.finally(() => {
      if (sequence === providerSwitchSequence.current) {
        setIsProviderSwitching(false);
      }
    });
  };

  // Segmented brain picker: "On my device" is the built-in provider; "Cloud
  // provider" is any other supported provider. Switching restores the user's
  // last valid cloud choice rather than a hidden/stale provider.
  const brainMode: "device" | "cloud" = isBuiltin ? "device" : "cloud";
  const handleBrainModeChange = (mode: "device" | "cloud") => {
    if (mode === "device") {
      if (!isBuiltin) handleProviderSelect(BUILTIN_PROVIDER_ID);
      return;
    }
    if (!isBuiltin) return;

    const target =
      lastCloudProviderId &&
      cloudProviderOptions.some(
        (option) => option.value === lastCloudProviderId,
      )
        ? lastCloudProviderId
        : cloudProviderOptions[0]?.value;
    if (target) handleProviderSelect(target);
  };

  /** Persist a pending API-key edit. Trims (a trailing space from a paste was
   *  stored verbatim and 401'd with no hint) and no-ops when nothing changed,
   *  so it is safe to call from both blur and the test button. */
  const persistApiKeyDraft = async () => {
    const trimmed = apiKey.trim();
    const stored = settings?.post_process_api_keys?.[selectedProviderId] ?? "";
    if (trimmed !== apiKey) setApiKey(trimmed);
    if (trimmed === stored) return;
    await updatePostProcessApiKey(selectedProviderId, trimmed);
  };

  const handleApiKeyFocus = () => {
    editingCredentialField.current = "apiKey";
  };

  const handleApiKeyBlur = async () => {
    editingCredentialField.current = null;
    await persistApiKeyDraft();
  };

  const handleBaseUrlFocus = () => {
    editingCredentialField.current = "baseUrl";
  };

  const handleBaseUrlBlur = async () => {
    editingCredentialField.current = null;
    // Base URLs are shared with AI cleanup, so use the hardened store action:
    // it checks Result.status, clears the now-invalid model, refreshes
    // readiness, and reports failures — instead of a silent bypass write.
    await updatePostProcessBaseUrl(selectedProviderId, baseUrl);
  };

  const setAndRefresh = async (promise: Promise<unknown>) => {
    await promise;
    await refreshSettings();
  };

  const currentTtsSpeed = settings?.assistant_tts_speed ?? 1;

  // Persist a playback speed (preset or typed). The backend clamps to 0.25–4.
  const commitTtsSpeed = async (value: number) => {
    const clamped = Math.min(4, Math.max(0.25, value));
    setTtsSpeedInput(String(clamped));
    await setAndRefresh(commands.setAssistantTtsSpeed(clamped));
  };

  const handleTtsSpeedBlur = () => {
    const parsed = parseFloat(ttsSpeedInput);
    if (Number.isFinite(parsed)) {
      void queueTtsTask(() => commitTtsSpeed(parsed));
    } else {
      // Revert an unparseable entry to the persisted value.
      setTtsSpeedInput(String(currentTtsSpeed));
    }
  };

  // --- Web search ---------------------------------------------------------
  const webSearchProvider = settings?.assistant_web_search_provider ?? "serper";
  const webSearchEnabled = settings?.assistant_web_search_enabled ?? false;
  const webSearchNeedsKey =
    webSearchProvider === "serper" ||
    webSearchProvider === "brave" ||
    webSearchProvider === "tavily" ||
    webSearchProvider === "exa" ||
    webSearchProvider === "serpapi" ||
    webSearchProvider === "tinyfish";

  // Sync the API-key field to the selected provider's stored key.
  useEffect(() => {
    setWebSearchApiKey(
      settings?.web_search_api_keys?.[webSearchProvider] ?? "",
    );
  }, [settings, webSearchProvider]);

  const handleWebSearchApiKeyBlur = async () => {
    await commands.setAssistantWebSearchApiKey(
      webSearchProvider,
      webSearchApiKey,
    );
    await refreshSettings();
  };

  // A "reachable" result belongs to the provider and model it was measured
  // against. Changing either invalidates it, so clear it rather than leave a
  // green line vouching for a configuration nobody tested.
  useEffect(() => {
    setBrainTest("idle");
    setBrainTestMsg(null);
  }, [selectedProviderId, model]);

  /** Send one throwaway question to the configured brain and report back.
   *  On the built-in engine this includes loading the model, so it can take
   *  tens of seconds the first time — the button stays in its "testing" state
   *  rather than pretending to have finished. */
  const handleTestBrain = async () => {
    setBrainTest("testing");
    setBrainTestMsg(null);
    try {
      // The test reads the key from the backend, and the field persists on
      // blur — so pasting a key and going straight for this button could test
      // the previous one. Flush the draft first; it no-ops when unchanged.
      // Mirrors what the TTS test already does with `persistRemoteTtsDraft`.
      await persistApiKeyDraft();
      const res = await commands.assistantTestConnection();
      if (res.status === "error") {
        setBrainTest("error");
        setBrainTestMsg(res.error);
        return;
      }
      setBrainTest("ok");
      setBrainTestMsg(res.data);
    } catch (e) {
      setBrainTest("error");
      setBrainTestMsg(String(e));
    }
  };

  const handleTestWebSearch = async () => {
    setWebSearchTest("testing");
    setWebSearchTestMsg(null);
    try {
      const res = await commands.assistantTestWebSearch(
        "who is the prime minister of canada",
      );
      if (res.status === "error") {
        setWebSearchTest("error");
        setWebSearchTestMsg(res.error);
        return;
      }
      setWebSearchTest("ok");
      setWebSearchTestMsg(
        t("settings.assistant.webSearch.testResult", {
          count: res.data.length,
        }),
      );
      setTimeout(() => setWebSearchTest("idle"), 4000);
    } catch (e) {
      setWebSearchTest("error");
      setWebSearchTestMsg(String(e));
    }
  };

  // Cloud provider form (Provider → Base URL where needed → API key → Model),
  // per the §4.0 consistency contract. Shared shape with Dictation's AI-cleanup.
  const cloudProviderForm = (
    <>
      <SettingContainer
        title={t("settings.assistant.provider.providerLabel")}
        layout="stacked"
        grouped={true}
      >
        <ProviderSelect
          options={cloudProviderOptions}
          value={selectedProviderId}
          onChange={handleProviderSelect}
          disabled={isProviderSwitching}
          modelsFor="assistant"
        />
      </SettingContainer>

      {selectedProvider?.allow_base_url_edit && (
        <SettingContainer
          title={t("settings.assistant.provider.baseUrlLabel")}
          info={t("settings.assistant.provider.baseUrlDescription")}
          layout="horizontal"
          grouped={true}
        >
          <Input
            type="text"
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            onFocus={handleBaseUrlFocus}
            onBlur={handleBaseUrlBlur}
            placeholder="https://my-resource.openai.azure.com/openai/v1"
            className="w-[340px]"
          />
        </SettingContainer>
      )}

      <SettingContainer
        title={t("settings.assistant.provider.apiKeyLabel")}
        info={t("settings.assistant.provider.apiKeyDescription")}
        layout="horizontal"
        grouped={true}
      >
        <Input
          type="password"
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
          onFocus={handleApiKeyFocus}
          onBlur={handleApiKeyBlur}
          placeholder={t("settings.assistant.provider.apiKeyPlaceholder")}
          className="w-[340px]"
        />
      </SettingContainer>

      <SettingContainer
        title={t("settings.assistant.provider.modelLabel")}
        layout="horizontal"
        grouped={true}
      >
        <div className="flex flex-col items-end gap-1">
          <LoadableSelect
            value={model}
            options={assistantModelOptions}
            onCommit={(v) => void handleAssistantModelChange(v)}
            onLoad={handleLoadAssistantModels}
            loading={modelsLoading}
            error={modelsError}
            placeholder={t(
              "settings.assistant.provider.modelSearchPlaceholder",
            )}
            loadLabel={t("settings.assistant.provider.loadModels")}
          />
        </div>
      </SettingContainer>
    </>
  );

  // On-device (built-in local engine) brain form: pick a downloaded model or
  // open the catalog to download one; context window + unload timeout fold.
  const deviceProviderForm = (
    <>
      <SettingContainer
        title={t("settings.assistant.provider.modelLabel")}
        layout="horizontal"
        grouped={true}
      >
        <div className="flex flex-col items-end gap-1">
          {/* The same picker the Assistant page uses, limited to this
              computer; its footer jumps to the catalog below. */}
          <LlmModelPicker
            role="assistant"
            scope="device"
            onBrowse={onOpenLlmCatalog}
          />
          {engineStatus.active ? (
            <div className="flex w-full max-w-[360px] flex-col items-end gap-1">
              <span className="inline-flex items-center gap-1.5 text-xs text-muted">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                {engineStatus.phase === "extracting"
                  ? t("settings.assistant.provider.builtinEngineExtracting")
                  : engineStatus.total > 0
                    ? t(
                        "settings.assistant.provider.builtinEngineDownloading",
                        {
                          percent: engineStatus.pct,
                        },
                      )
                    : t("settings.assistant.provider.builtinEnginePreparing")}
              </span>
              <div className="h-1.5 w-full overflow-hidden rounded-full bg-hairline-strong">
                <div
                  className={`h-full rounded-full bg-accent ${
                    engineStatus.total > 0
                      ? "transition-[width] duration-200"
                      : "w-full animate-pulse"
                  }`}
                  style={
                    engineStatus.total > 0
                      ? { width: `${engineStatus.pct}%` }
                      : undefined
                  }
                />
              </div>
            </div>
          ) : (
            localLlmStatus &&
            !localLlmStatus.engine_present && (
              <span className="text-xs text-amber-500 max-w-[360px] text-right">
                {t("settings.assistant.provider.builtinEngineMissing")}
              </span>
            )
          )}
        </div>
      </SettingContainer>

      <SettingContainer
        title={t("settings.assistant.provider.contextSizeLabel")}
        info={t("settings.assistant.provider.contextSizeDescription")}
        layout="horizontal"
        grouped={true}
      >
        <Input
          type="number"
          min={512}
          max={32768}
          step={512}
          value={contextSize}
          onChange={(e) => setContextSize(e.target.value)}
          onBlur={handleContextSizeBlur}
          className="w-[120px]"
        />
      </SettingContainer>

      <SettingContainer
        title={t("settings.assistant.provider.unloadTimeoutLabel")}
        info={t("settings.assistant.provider.unloadTimeoutDescription")}
        layout="horizontal"
        grouped={true}
      >
        <Dropdown
          options={llmUnloadOptions}
          selectedValue={
            settings?.local_llm_unload_timeout ?? ("min5" as ModelUnloadTimeout)
          }
          onSelect={(value) =>
            setAndRefresh(
              commands.setLocalLlmUnloadTimeout(value as ModelUnloadTimeout),
            )
          }
          className="min-w-[200px]"
        />
      </SettingContainer>
    </>
  );

  /** The master switch, and the one group that outlives it: "Generate with
   *  Flow" and AI Correction run on the same provider and model, so the brain
   *  picker stays reachable even with the assistant switched off. */
  const masterSwitchGroup = (
    <SettingsGroup
      title={groupTitle(t("settings.assistant.enable.title"))}
      icon={Power}
    >
      <ToggleSwitch
        checked={assistantEnabled}
        onChange={(checked) =>
          setAndRefresh(commands.setAssistantEnabled(checked))
        }
        label={t("settings.assistant.enable.label")}
        description={t("settings.assistant.enable.description")}
        grouped={true}
      />
    </SettingsGroup>
  );

  const brainGroup = (
    <SettingsGroup
      title={groupTitle(t("settings.assistant.brain.title"))}
      icon={Sparkles}
    >
      <SettingContainer
        title={t("settings.assistant.brain.whereLabel")}
        layout="horizontal"
        grouped={true}
      >
        <ProviderModeToggle
          mode={brainMode}
          onChange={handleBrainModeChange}
          disabled={isProviderSwitching}
        />
      </SettingContainer>
      {brainMode === "device" ? deviceProviderForm : cloudProviderForm}
      {brainMode === "cloud" && (
        <ThinkingLevelSetting
          job="assistant"
          providerId={settings?.assistant_provider_id}
          grouped={true}
        />
      )}

      {/* Last row of the group on purpose: it tests whatever the rows above
          resolved to, in either mode. A wrong key, an empty balance, or a model
          id this endpoint doesn't serve all fail identically the first time the
          user talks to the assistant — and that failure arrives as silence. */}
      <SettingContainer
        title={t("settings.assistant.brain.testLabel")}
        info={t("settings.assistant.brain.testDescription")}
        layout="horizontal"
        grouped={true}
      >
        <div className="flex flex-col items-end gap-1">
          <button
            type="button"
            onClick={handleTestBrain}
            disabled={brainTest === "testing" || isProviderSwitching}
            className="flex items-center gap-2 px-3 py-1.5 rounded-lg border border-hairline-strong bg-surface hover:bg-surface-strong disabled:opacity-50 disabled:cursor-not-allowed text-[13px] font-medium cursor-pointer transition-colors"
          >
            {brainTest === "testing" ? (
              <Loader2 size={14} className="animate-spin" />
            ) : (
              <PlugZap size={14} />
            )}
            {brainTest === "testing"
              ? t("settings.assistant.brain.testing")
              : t("settings.assistant.brain.testButton")}
          </button>
          {brainTestMsg && (
            <span
              className={`text-xs max-w-[360px] text-right break-words ${
                brainTest === "error" ? "text-error" : "text-muted-soft"
              }`}
            >
              {brainTestMsg}
            </span>
          )}
        </div>
      </SettingContainer>
    </SettingsGroup>
  );

  // Assistant off: the page shrinks to the switch that turns it back on, plus
  // the shared brain picker. Everything below belongs to the assistant itself.
  // A caller that asked for specific sections gets exactly those.
  if (!assistantEnabled && !sections) {
    return (
      <div className="max-w-3xl w-full mx-auto space-y-8">
        {masterSwitchGroup}
        {brainGroup}
      </div>
    );
  }

  return (
    <div className="w-full space-y-8">
      {show("master") && masterSwitchGroup}

      {/* Hotkeys ---------------------------------------------------------- */}
      {show("shortcuts") && (
        <SettingsGroup
          title={groupTitle(t("settings.assistant.shortcuts.title"))}
          icon={Keyboard}
        >
          <ShortcutInput
            shortcutId="assistant"
            grouped={true}
            icon={Sparkles}
            tone="teal"
          />
          {/* The call's own key, listed next to the quick ask so the split between
            the two features is visible in the one place people go looking for
            it. They are separate features with separate lifetimes: one answers a
            question and closes, the other holds a conversation. */}
          <ShortcutInput
            shortcutId="assistant_call"
            grouped={true}
            icon={AudioLines}
            tone="indigo"
          />
          <PushToTalk grouped={true} />
        </SettingsGroup>
      )}

      {/* Brain picker ----------------------------------------------------- */}
      {show("brain") && brainGroup}

      {/* Voice output ----------------------------------------------------- */}
      {show("voice") && (
        <SettingsGroup
          title={groupTitle(t("settings.assistant.tts.title"))}
          icon={Volume2}
        >
          <ToggleSwitch
            checked={settings?.assistant_tts_enabled ?? false}
            onChange={(checked) =>
              setAndRefresh(commands.setAssistantTtsEnabled(checked))
            }
            label={t("settings.assistant.tts.enableLabel")}
            description={t("settings.assistant.tts.enableDescription")}
            grouped={true}
          />
          {ttsEnabled && (
            <>
              <SettingContainer
                title={t("settings.assistant.tts.engineLabel")}
                layout="stacked"
                grouped={true}
              >
                {/* Two groups, one choice: where the voice runs is the first
                  thing people decide, so it is the first thing the grid
                  says. Both groups share one value; the unselected group
                  simply has no ringed tile. What each group means sits behind
                  its (i), so the grid reads as two labels and the tiles. */}
                <div className="space-y-4">
                  {(
                    [
                      {
                        key: "local",
                        engines: TTS_ENGINES.filter((e) => e.local),
                      },
                      {
                        key: "cloud",
                        engines: TTS_ENGINES.filter((e) => !e.local),
                      },
                    ] as const
                  ).map(({ key, engines }) => {
                    const heading =
                      key === "local"
                        ? t("settings.assistant.tts.sectionLocal")
                        : t("settings.assistant.tts.sectionCloud");
                    return (
                      <section key={key} className="space-y-2">
                        <div className="flex items-center gap-1">
                          <h4 className="text-[0.8125rem] font-medium text-muted">
                            {heading}
                          </h4>
                          <InfoTip
                            text={
                              key === "local"
                                ? t("settings.assistant.tts.sectionLocalHint")
                                : t("settings.assistant.tts.sectionCloudHint")
                            }
                          />
                        </div>
                        <LogoChoice
                          label={heading}
                          minTile="9.5rem"
                          readyLabel={
                            key === "local"
                              ? t("settings.assistant.tts.readyLabel")
                              : t("assistantPage.cards.keySaved")
                          }
                          options={engines.map((engine) => ({
                            value: engine.id,
                            label: t(`voiceEngines.names.${engine.id}`),
                            hint: t(`voiceEngines.${engine.id}`),
                            title: t(
                              `settings.assistant.tts.engines.${engine.id}`,
                            ),
                            ready: isNativeEngine(engine.id)
                              ? nativeEngineReady(engine.id)
                              : !!engine.local ||
                                !ttsNeedsSetup(settings, engine.id),
                            icon: (
                              <ProviderTile
                                id={engine.id}
                                kind="tts"
                                size="md"
                              />
                            ),
                          }))}
                          value={settings?.assistant_tts_engine ?? "kokoro"}
                          onChange={(engine) => {
                            void queueTtsTask(async () => {
                              await setAndRefresh(
                                commands.setAssistantTtsEngine(engine),
                              );
                            });
                          }}
                          disabled={!settings?.assistant_tts_enabled}
                        />
                      </section>
                    );
                  })}
                </div>
              </SettingContainer>

              {(settings?.assistant_tts_engine ?? "kokoro") === "kokoro" && (
                <>
                  <SettingContainer
                    title={t("settings.assistant.tts.runsOnLabel")}
                    description={runsOnStatus}
                    descriptionMode="inline"
                    layout="horizontal"
                    grouped={true}
                  >
                    <Dropdown
                      options={(["auto", "gpu", "cpu"] as const).map(
                        (device) => ({
                          value: device,
                          label: t(`settings.assistant.tts.runsOn.${device}`),
                        }),
                      )}
                      selectedValue={kokoroDevice}
                      onSelect={(device) =>
                        setAndRefresh(
                          commands.setAssistantTtsKokoroDevice(device),
                        )
                      }
                      disabled={!settings?.assistant_tts_enabled}
                      className="min-w-[340px]"
                    />
                  </SettingContainer>

                  {offerProcessorVoice && (
                    <NativeVoicePackRow
                      modelId={KOKORO_NATIVE_MODEL_ID}
                      title={t("settings.assistant.tts.kokoroNativeLabel")}
                      description={(size) =>
                        t("settings.assistant.tts.kokoroNativeDescription", {
                          size,
                        })
                      }
                      unsupported={
                        localVoice ? !localVoice.native_supported : false
                      }
                      disabled={!settings?.assistant_tts_enabled}
                    />
                  )}

                  {suggestKitten && (
                    <SettingContainer
                      title={t("settings.assistant.tts.kittenSuggestLabel")}
                      description={t(
                        "settings.assistant.tts.kittenSuggestDescription",
                      )}
                      descriptionMode="inline"
                      layout="horizontal"
                      grouped={true}
                    >
                      <Button
                        variant="secondary"
                        size="sm"
                        onClick={() => {
                          void queueTtsTask(async () => {
                            await setAndRefresh(
                              commands.setAssistantTtsEngine("kitten"),
                            );
                          });
                        }}
                      >
                        {t("settings.assistant.tts.kittenSuggestButton")}
                      </Button>
                    </SettingContainer>
                  )}

                  {!kokoroOnProcessor && (
                    <SettingContainer
                      title={t("settings.assistant.tts.kokoroSetupLabel")}
                      description={t(
                        "settings.assistant.tts.kokoroSetupDescription",
                      )}
                      descriptionMode="inline"
                      layout="horizontal"
                      grouped={true}
                    >
                      <div className="flex min-w-[340px] justify-end">
                        {/* Weights already on disk read as Ready even while
                          they load: loading them from the cache fires the
                          same progress events as a download, and switching
                          engines drops the model, so every return to Kokoro
                          used to show "Downloading Kokoro" for a file that
                          was already here. The Test button shows the wait. */}
                        {kokoroPrepared ||
                        kokoroStatus === "ready" ||
                        kokoroStatus === "speaking" ? (
                          <span className="inline-flex items-center gap-1.5 text-[13px] font-medium text-accent">
                            <Check className="h-4 w-4" />
                            {t("settings.assistant.tts.kokoroReady")}
                          </span>
                        ) : kokoroStatus === "loading" ? (
                          <div className="w-full max-w-[260px] space-y-1.5">
                            <div className="flex items-center justify-between gap-3 text-xs text-muted">
                              <span className="inline-flex items-center gap-1.5">
                                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                                {t("settings.assistant.tts.kokoroDownloading")}
                              </span>
                              <span className="tabular-nums">
                                {kokoroProgress}%
                              </span>
                            </div>
                            <div className="h-1.5 overflow-hidden rounded-full bg-hairline-strong">
                              <div
                                className="h-full rounded-full bg-accent transition-[width] duration-200"
                                style={{ width: `${kokoroProgress}%` }}
                              />
                            </div>
                          </div>
                        ) : kokoroPrepared === null ? null : (
                          <div className="flex flex-col items-end gap-1.5">
                            <Button
                              variant={
                                kokoroError ? "secondary" : "primary-soft"
                              }
                              size="sm"
                              onClick={() => void handlePrepareKokoro()}
                            >
                              <Download className="h-3.5 w-3.5" />
                              {kokoroError
                                ? t("settings.assistant.tts.kokoroRetry")
                                : t("settings.assistant.tts.kokoroDownload")}
                            </Button>
                            {kokoroError && (
                              <span className="text-xs text-error">
                                {t("settings.assistant.tts.downloadError")}
                              </span>
                            )}
                          </div>
                        )}
                      </div>
                    </SettingContainer>
                  )}

                  <SettingContainer
                    title={t("settings.assistant.tts.voiceLabel")}
                    layout="horizontal"
                    grouped={true}
                  >
                    <Dropdown
                      options={KOKORO_VOICES.map((voice) => ({
                        value: voice.value,
                        label: t(
                          `settings.assistant.tts.kokoroVoices.${voice.kind}`,
                          { name: voice.name },
                        ),
                      }))}
                      selectedValue={
                        settings?.assistant_tts_voice ?? "af_heart"
                      }
                      onSelect={(voice) =>
                        setAndRefresh(commands.setAssistantTtsVoice(voice))
                      }
                      disabled={!settings?.assistant_tts_enabled}
                      className="min-w-[340px]"
                    />
                  </SettingContainer>
                </>
              )}

              {isNativeEngine(ttsEngine) && (
                <NativeEngineRows
                  engine={ttsEngine}
                  model={settings?.assistant_tts_model ?? ""}
                  voice={settings?.assistant_tts_remote_voice ?? ""}
                  unsupported={
                    localVoice ? !localVoice.native_supported : false
                  }
                  disabled={!settings?.assistant_tts_enabled}
                  onModel={(model) =>
                    void queueTtsTask(async () => {
                      await setAndRefresh(commands.setAssistantTtsModel(model));
                    })
                  }
                  onVoice={(voice) =>
                    void queueTtsTask(async () => {
                      await setAndRefresh(
                        commands.setAssistantTtsRemoteVoice(voice),
                      );
                    })
                  }
                />
              )}

              {ttsSpec && !ttsSpec.local && (
                <>
                  {ttsSpec.url && (
                    <SettingContainer
                      title={
                        ttsEngine === "azure"
                          ? t("settings.assistant.tts.azureBaseUrlLabel")
                          : t("settings.assistant.tts.baseUrlLabel")
                      }
                      info={
                        ttsEngine === "azure"
                          ? t("settings.assistant.tts.azureBaseUrlDescription")
                          : t("settings.assistant.tts.baseUrlDescription")
                      }
                      layout="horizontal"
                      grouped={true}
                    >
                      <Input
                        type="text"
                        value={ttsBaseUrl}
                        onChange={(e) => setTtsBaseUrl(e.target.value)}
                        onBlur={() => {
                          void queueTtsTask(async () => {
                            await setAndRefresh(
                              commands.setAssistantTtsBaseUrl(ttsBaseUrl),
                            );
                          });
                        }}
                        placeholder={ttsSpec.url.example}
                        spellCheck={false}
                        className="w-[340px]"
                      />
                    </SettingContainer>
                  )}
                  <SettingContainer
                    title={
                      ttsSpec.key === "optional"
                        ? t("pickers.setup.apiKeyOptional")
                        : t("settings.assistant.tts.apiKeyLabel")
                    }
                    info={
                      ttsSpec.key === "optional"
                        ? t("settings.assistant.tts.apiKeyOptionalDescription")
                        : t("settings.assistant.tts.apiKeyDescription", {
                            provider: ttsEngineName,
                          })
                    }
                    layout="horizontal"
                    grouped={true}
                  >
                    <div className="flex w-[340px] flex-col items-end gap-1">
                      <Input
                        type="password"
                        value={ttsApiKey}
                        onChange={(e) => setTtsApiKey(e.target.value)}
                        onBlur={() => {
                          void queueTtsTask(async () => {
                            await setAndRefresh(
                              commands.setAssistantTtsApiKey(ttsApiKey),
                            );
                          });
                        }}
                        autoComplete="off"
                        spellCheck={false}
                        className="w-full"
                      />
                      {ttsSpec.keyUrl && !ttsApiKey.trim() && (
                        <button
                          type="button"
                          onClick={() =>
                            void openUrl(ttsSpec.keyUrl!).catch(() => {})
                          }
                          className="inline-flex cursor-pointer items-center gap-0.5 rounded text-xs font-medium text-muted transition-colors hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
                        >
                          {t("pickers.setup.getKey", {
                            site: hostOf(ttsSpec.keyUrl),
                          })}
                          <ArrowUpRight className="h-3 w-3" aria-hidden />
                        </button>
                      )}
                    </div>
                  </SettingContainer>
                  {ttsSpec.model && (
                    <SettingContainer
                      title={t("settings.assistant.tts.modelLabel")}
                      info={t("settings.assistant.tts.modelDescription")}
                      layout="horizontal"
                      grouped={true}
                    >
                      <LoadableSelect
                        value={ttsModel}
                        options={ttsModelOptions}
                        onCommit={(v) => {
                          setTtsModel(v);
                          void queueTtsTask(async () => {
                            await setAndRefresh(
                              commands.setAssistantTtsModel(v),
                            );
                          });
                        }}
                        onLoad={handleLoadTtsModels}
                        loading={ttsModelsLoading}
                        error={ttsModelsError}
                        placeholder={ttsSpec.model.example}
                        loadLabel={t("settings.assistant.tts.loadModels")}
                        formatCreateLabel={(input) =>
                          t("settings.assistant.tts.modelsUse", {
                            model: input,
                          })
                        }
                      />
                    </SettingContainer>
                  )}
                  <SettingContainer
                    title={
                      ttsEngine === "elevenlabs"
                        ? t("settings.assistant.tts.elevenVoiceLabel")
                        : t("settings.assistant.tts.voiceLabel")
                    }
                    info={ttsVoiceHelp}
                    layout="horizontal"
                    grouped={true}
                  >
                    <LoadableSelect
                      value={ttsRemoteVoice}
                      options={ttsVoiceOptions}
                      onCommit={(v) => {
                        setTtsRemoteVoice(v);
                        void queueTtsTask(async () => {
                          await setAndRefresh(
                            commands.setAssistantTtsRemoteVoice(v),
                          );
                        });
                      }}
                      onLoad={handleLoadTtsVoices}
                      loading={ttsVoicesLoading}
                      error={ttsVoicesError}
                      placeholder={ttsVoicePlaceholder}
                      loadLabel={t("settings.assistant.tts.loadVoices")}
                      formatCreateLabel={(input) =>
                        t("settings.assistant.tts.voicesUse", { voice: input })
                      }
                    />
                  </SettingContainer>
                </>
              )}

              {/* ElevenLabs only: no other engine reads this setting. */}
              {ttsEngine === "elevenlabs" && (
                <ElevenLabsExpressiveness
                  stability={settings?.assistant_tts_elevenlabs_stability}
                  onCommit={(stability) =>
                    // Queued with the other voice saves, so pressing Test
                    // right after a drag hears the new setting.
                    queueTtsTask(async () => {
                      try {
                        await runTtsCommand(
                          commands.setAssistantTtsElevenlabsStability(
                            stability,
                          ),
                        );
                      } catch (error) {
                        toast.error(String(error));
                      }
                      await refreshSettings();
                    })
                  }
                />
              )}

              {/* ElevenLabs only, and only meaningful on a model that performs
                tags (v3 and later). On an older model the switch stays visible
                but disabled, saying which models it needs, so the feature can
                be found; the backend ignores it there either way. */}
              {ttsEngine === "elevenlabs" && (
                <>
                  <ToggleSwitch
                    checked={audioTagsOn && audioTagsModelOk}
                    disabled={!audioTagsModelOk}
                    onChange={(checked) =>
                      void queueTtsTask(() =>
                        setAndRefresh(
                          commands.setAssistantTtsElevenlabsAudioTags(checked),
                        ),
                      )
                    }
                    label={t("settings.assistant.tts.audioTagsLabel")}
                    description={
                      audioTagsModelOk
                        ? t("settings.assistant.tts.audioTagsDescription")
                        : t("settings.assistant.tts.audioTagsNeedsModel")
                    }
                    info={t("settings.assistant.tts.audioTagsInfo")}
                    grouped={true}
                  />
                  {audioTagsActive && (
                    <Slider
                      value={Math.max(
                        0,
                        AUDIO_TAG_INTENSITIES.indexOf(audioTagIntensity),
                      )}
                      onChange={(index) =>
                        queueTtsTask(() =>
                          setAndRefresh(
                            commands.setAssistantTtsElevenlabsAudioTagIntensity(
                              AUDIO_TAG_INTENSITIES[Math.round(index)] ??
                                "balanced",
                            ),
                          ),
                        )
                      }
                      min={0}
                      max={AUDIO_TAG_INTENSITIES.length - 1}
                      step={1}
                      label={t("settings.assistant.tts.audioTagIntensityLabel")}
                      description={t(
                        "settings.assistant.tts.audioTagIntensityDescription",
                      )}
                      grouped={true}
                      controlClassName="w-[300px]"
                      valueClassName="min-w-[5.5rem] whitespace-nowrap ps-2"
                      formatValue={(index) =>
                        t(
                          `settings.assistant.tts.audioTagIntensityLevels.${
                            AUDIO_TAG_INTENSITIES[Math.round(index)] ??
                            "balanced"
                          }`,
                        )
                      }
                    />
                  )}
                </>
              )}

              <SettingContainer
                title={t("settings.assistant.tts.speedLabel")}
                info={ttsSpeedHelp}
                layout="horizontal"
                grouped={true}
              >
                <div className="flex items-center gap-1.5">
                  {TTS_SPEED_PRESETS.map((preset) => {
                    const active = Math.abs(currentTtsSpeed - preset) < 0.001;
                    return (
                      <button
                        key={preset}
                        type="button"
                        onClick={() => {
                          void queueTtsTask(() => commitTtsSpeed(preset));
                        }}
                        disabled={
                          !settings?.assistant_tts_enabled || !ttsHasSpeed
                        }
                        className={`px-2.5 py-1 text-[13px] font-medium rounded-md transition-colors cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed ${
                          active
                            ? "bg-accent/12 text-accent"
                            : "bg-surface-strong text-muted hover:text-ink"
                        }`}
                      >
                        {t("settings.assistant.tts.speedValue", {
                          value: preset,
                        })}
                      </button>
                    );
                  })}
                  <Input
                    type="number"
                    value={ttsSpeedInput}
                    onChange={(e) => setTtsSpeedInput(e.target.value)}
                    onBlur={handleTtsSpeedBlur}
                    min="0.25"
                    max="4"
                    step="0.1"
                    disabled={!settings?.assistant_tts_enabled || !ttsHasSpeed}
                    aria-label={t("settings.assistant.tts.speedCustomLabel")}
                    className="w-20"
                  />
                </div>
              </SettingContainer>

              {/* Its own control, deliberately. Spoken replies used to be gained
                by the feedback-sound slider, which is greyed out whenever
                feedback sounds are off — so turning the beeps down once and
                then switching them off left the voice quiet with nothing to
                turn it back up. Unlike Speed it stays enabled with spoken
                replies off, because a call turns them on regardless. */}
              <Slider
                value={settings?.assistant_tts_volume ?? 1}
                onChange={(value) =>
                  setAndRefresh(commands.setAssistantTtsVolume(value))
                }
                // A call that is speaking hears the change while it is dragged.
                liveCommitMs={150}
                min={0}
                max={1}
                step={0.05}
                label={t("settings.assistant.tts.volumeLabel")}
                info={t("settings.assistant.tts.volumeDescription")}
                grouped={true}
                controlClassName="w-[200px]"
                formatValue={(v) => `${Math.round(v * 100)}%`}
              />

              <SettingContainer
                title={t("settings.assistant.tts.testLabel")}
                layout="horizontal"
                grouped={true}
              >
                <div className="flex flex-col items-end gap-1">
                  <button
                    type="button"
                    onClick={handleTestTts}
                    disabled={
                      !settings?.assistant_tts_enabled ||
                      testState === "testing" ||
                      kokoroDownloading
                    }
                    className="flex items-center gap-2 px-3 py-1.5 rounded-lg border border-hairline-strong bg-surface hover:bg-surface-strong disabled:opacity-50 disabled:cursor-not-allowed text-[13px] font-medium cursor-pointer transition-colors"
                  >
                    <Volume2 size={14} />
                    {testState === "testing"
                      ? t("settings.assistant.tts.testing")
                      : testState === "ok"
                        ? t("settings.assistant.tts.testOk")
                        : t("settings.assistant.tts.testButton")}
                  </button>
                  {testState === "error" && testError && (
                    <span className="text-xs text-error max-w-[360px] text-right break-words">
                      {testError}
                    </span>
                  )}
                </div>
              </SettingContainer>

              <ToggleSwitch
                checked={settings?.assistant_tts_stop_on_dictation ?? false}
                onChange={(checked) =>
                  setAndRefresh(
                    commands.setAssistantTtsStopOnDictation(checked),
                  )
                }
                label={t("settings.assistant.tts.stopOnDictationLabel")}
                grouped={true}
              />
              {(settings?.assistant_tts_engine ?? "kokoro") === "kokoro" &&
                !kokoroOnProcessor && (
                  <SettingContainer
                    title={t("settings.assistant.tts.dtypeLabel")}
                    info={t("settings.assistant.tts.dtypeDescription")}
                    layout="horizontal"
                    grouped={true}
                  >
                    <Dropdown
                      options={KOKORO_DTYPES.map((value) => ({
                        value,
                        label: t(`settings.assistant.tts.dtypes.${value}`),
                      }))}
                      selectedValue={
                        settings?.assistant_tts_kokoro_dtype ?? "fp32"
                      }
                      onSelect={(dtype) =>
                        setAndRefresh(
                          commands.setAssistantTtsKokoroDtype(dtype),
                        )
                      }
                      disabled={!settings?.assistant_tts_enabled}
                    />
                  </SettingContainer>
                )}
            </>
          )}
        </SettingsGroup>
      )}

      {/* Web search ------------------------------------------------------- */}
      {show("webSearch") && (
        <SettingsGroup
          title={groupTitle(t("settings.assistant.webSearch.title"))}
          icon={Globe}
        >
          <ToggleSwitch
            checked={webSearchEnabled}
            onChange={(checked) =>
              setAndRefresh(commands.setAssistantWebSearchEnabled(checked))
            }
            label={t("settings.assistant.webSearch.enableLabel")}
            description={t("settings.assistant.webSearch.enableDescription")}
            grouped={true}
          />
          {webSearchEnabled && (
            <>
              {selectedProviderId === "openrouter" && (
                <ToggleSwitch
                  checked={
                    settings?.assistant_prefer_provider_web_search ?? true
                  }
                  onChange={(checked) =>
                    setAndRefresh(
                      commands.setAssistantPreferProviderWebSearch(checked),
                    )
                  }
                  label={t(
                    "settings.assistant.webSearch.openRouterNativeLabel",
                  )}
                  info={t(
                    "settings.assistant.webSearch.openRouterNativeDescription",
                  )}
                  grouped={true}
                />
              )}
              <SettingContainer
                title={t("settings.assistant.webSearch.providerLabel")}
                info={t("settings.assistant.webSearch.providerDescription")}
                layout="stacked"
                grouped={true}
              >
                <LogoChoice
                  label={t("settings.assistant.webSearch.providerLabel")}
                  minTile="9rem"
                  readyLabel={t("assistantPage.cards.keySaved")}
                  options={(
                    [
                      "tinyfish",
                      "serper",
                      "brave",
                      "tavily",
                      "exa",
                      "serpapi",
                    ] as const
                  ).map((provider) => ({
                    value: provider,
                    label: t(
                      `settings.assistant.webSearch.providers.${provider}`,
                    ).replace(/\s*\([^)]*\)\s*$/, ""),
                    hint: t(`assistantPage.cards.webSearch.hints.${provider}`),
                    ready: !!settings?.web_search_api_keys?.[provider]?.trim(),
                    icon: (
                      <ProviderTile id={provider} kind="search" size="md" />
                    ),
                  }))}
                  value={webSearchProvider}
                  onChange={(provider) =>
                    setAndRefresh(
                      commands.setAssistantWebSearchProvider(provider),
                    )
                  }
                  disabled={!webSearchEnabled}
                />
              </SettingContainer>

              {webSearchNeedsKey && (
                <SettingContainer
                  title={t("settings.assistant.webSearch.apiKeyLabel")}
                  layout="horizontal"
                  grouped={true}
                >
                  <Input
                    type="password"
                    value={webSearchApiKey}
                    onChange={(e) => setWebSearchApiKey(e.target.value)}
                    onBlur={handleWebSearchApiKeyBlur}
                    placeholder={t(
                      "settings.assistant.webSearch.apiKeyPlaceholder",
                    )}
                    className="min-w-[320px]"
                    disabled={!webSearchEnabled}
                  />
                </SettingContainer>
              )}

              <SettingContainer
                title={t("settings.assistant.webSearch.testLabel")}
                layout="horizontal"
                grouped={true}
              >
                <div className="flex flex-col items-end gap-1">
                  <button
                    type="button"
                    onClick={handleTestWebSearch}
                    disabled={!webSearchEnabled || webSearchTest === "testing"}
                    className="flex items-center gap-2 px-3 py-1.5 rounded-lg border border-hairline-strong bg-surface hover:bg-surface-strong disabled:opacity-50 disabled:cursor-not-allowed text-[13px] font-medium cursor-pointer transition-colors"
                  >
                    <Globe size={14} />
                    {webSearchTest === "testing"
                      ? t("settings.assistant.webSearch.testing")
                      : t("settings.assistant.webSearch.testButton")}
                  </button>
                  {webSearchTestMsg && (
                    <span
                      className={`text-xs max-w-[360px] text-right break-words ${
                        webSearchTest === "error"
                          ? "text-error"
                          : "text-muted-soft"
                      }`}
                    >
                      {webSearchTestMsg}
                    </span>
                  )}
                </div>
              </SettingContainer>

              <SettingContainer
                title={t("settings.assistant.webSearch.depthLabel")}
                info={t("settings.assistant.webSearch.depthDescription")}
                layout="horizontal"
                grouped={true}
              >
                <Dropdown
                  options={[
                    {
                      value: "low",
                      label: t("settings.assistant.webSearch.depthOptions.low"),
                    },
                    {
                      value: "medium",
                      label: t(
                        "settings.assistant.webSearch.depthOptions.medium",
                      ),
                    },
                    {
                      value: "high",
                      label: t(
                        "settings.assistant.webSearch.depthOptions.high",
                      ),
                    },
                  ]}
                  selectedValue={settings?.assistant_search_depth ?? "medium"}
                  onSelect={(depth) =>
                    setAndRefresh(
                      commands.setAssistantSearchDepth(
                        depth as AssistantSearchDepth,
                      ),
                    )
                  }
                  disabled={!webSearchEnabled}
                />
              </SettingContainer>

              {selectedProviderId === "builtin" && (
                <ToggleSwitch
                  checked={settings?.assistant_local_search_smart ?? false}
                  onChange={(checked) =>
                    setAndRefresh(
                      commands.setAssistantLocalSearchSmart(checked),
                    )
                  }
                  label={t("settings.assistant.webSearch.localSmartLabel")}
                  info={t("settings.assistant.webSearch.localSmartDescription")}
                  grouped={true}
                  disabled={!webSearchEnabled}
                />
              )}
            </>
          )}
        </SettingsGroup>
      )}

      {/* Reminders ---------------------------------------------------------
          Directly after search, because both are things the assistant does on
          your behalf rather than settings that change how it talks. */}
      {show("reminders") && <RemindersSettings />}

      {/* Panel appearance -------------------------------------------------- */}
      {show("appearance") && (
        <SettingsGroup
          title={groupTitle(t("settings.assistant.appearance.title"))}
          icon={PanelTop}
        >
          <SettingContainer
            title={t("settings.assistant.appearance.previewLabel")}
            layout="stacked"
            grouped={true}
          >
            <PanelPreview
              fontSize={settings?.assistant_font_size ?? "medium"}
              opacity={settings?.assistant_panel_opacity ?? 1}
            />
          </SettingContainer>
          <SettingContainer
            title={t("settings.assistant.appearance.fontSizeLabel")}
            layout="horizontal"
            grouped={true}
          >
            <Dropdown
              options={[
                {
                  value: "small",
                  label: t("settings.assistant.appearance.fontSizes.small"),
                },
                {
                  value: "medium",
                  label: t("settings.assistant.appearance.fontSizes.medium"),
                },
                {
                  value: "large",
                  label: t("settings.assistant.appearance.fontSizes.large"),
                },
                {
                  value: "extra_large",
                  label: t(
                    "settings.assistant.appearance.fontSizes.extraLarge",
                  ),
                },
              ]}
              selectedValue={settings?.assistant_font_size ?? "medium"}
              onSelect={(size) =>
                setAndRefresh(commands.setAssistantFontSize(size))
              }
            />
          </SettingContainer>
          {/* Only worth showing when there is a choice to make. On one monitor the
            row would be a dropdown with a single meaningful entry. */}
          {displays.length > 1 && (
            <SettingContainer
              title={t("settings.assistant.appearance.askDisplayLabel")}
              info={t("settings.assistant.appearance.askDisplayDescription")}
              layout="horizontal"
              grouped={true}
            >
              <Dropdown
                options={askDisplayOptions(displays, t)}
                selectedValue={askDisplayValue(settings?.assistant_ask_display)}
                onSelect={(display) =>
                  setAndRefresh(commands.setAssistantAskDisplay(display))
                }
              />
            </SettingContainer>
          )}
          <SettingContainer
            title={t("settings.assistant.appearance.askAnchorLabel")}
            info={t("settings.assistant.appearance.askAnchorDescription")}
            layout="horizontal"
            grouped={true}
          >
            <Dropdown
              options={[
                {
                  value: "center",
                  label: t("settings.assistant.appearance.askAnchors.center"),
                },
                {
                  value: "topcenter",
                  label: t("settings.assistant.appearance.askAnchors.top"),
                },
                {
                  value: "bottomcenter",
                  label: t("settings.assistant.appearance.askAnchors.bottom"),
                },
                {
                  value: "left",
                  label: t("settings.assistant.appearance.askAnchors.left"),
                },
                {
                  value: "right",
                  label: t("settings.assistant.appearance.askAnchors.right"),
                },
                // "Where I left it" is deliberately absent: the quick ask opens at
                // its dock zone every time, and a stored `custom` from an older
                // version reads as the default (the top).
              ]}
              selectedValue={
                settings?.assistant_ask_anchor === "custom"
                  ? "topcenter"
                  : (settings?.assistant_ask_anchor ?? "topcenter")
              }
              onSelect={(anchor) =>
                setAndRefresh(
                  commands.setAssistantAskAnchor(anchor as AskAnchor),
                )
              }
            />
          </SettingContainer>
          <Slider
            value={settings?.assistant_panel_opacity ?? 1}
            onChange={(value) =>
              setAndRefresh(commands.setAssistantPanelOpacity(value))
            }
            // An open panel follows the drag.
            liveCommitMs={150}
            min={0.5}
            max={1}
            step={0.05}
            label={t("settings.assistant.appearance.opacityLabel")}
            info={t("settings.assistant.appearance.opacityDescription")}
            grouped={true}
            controlClassName="w-[200px]"
            formatValue={(v) => `${Math.round(v * 100)}%`}
          />
        </SettingsGroup>
      )}

      {/* Reply behavior ---------------------------------------------------- */}
      {show("behavior") && (
        <SettingsGroup
          title={groupTitle(t("settings.assistant.behavior.title"))}
        >
          <SettingContainer
            title={t("settings.assistant.responseLength.label")}
            info={t("settings.assistant.responseLength.description")}
            layout="horizontal"
            grouped={true}
          >
            <Dropdown
              options={[
                {
                  value: "default",
                  label: t("settings.assistant.responseLength.options.default"),
                },
                {
                  value: "short",
                  label: t("settings.assistant.responseLength.options.short"),
                },
                {
                  value: "medium",
                  label: t("settings.assistant.responseLength.options.medium"),
                },
                {
                  value: "long",
                  label: t("settings.assistant.responseLength.options.long"),
                },
              ]}
              selectedValue={settings?.assistant_response_length ?? "default"}
              onSelect={(value) =>
                setAndRefresh(
                  commands.setAssistantResponseLength(
                    value as AssistantResponseLength,
                  ),
                )
              }
            />
          </SettingContainer>
          <SettingContainer
            title={t("settings.assistant.memory.label")}
            description={t("settings.assistant.memory.description")}
            layout="horizontal"
            grouped={true}
          >
            <Input
              type="number"
              min={0}
              max={200}
              value={historyLimit}
              onChange={(e) => setHistoryLimit(e.target.value)}
              onBlur={handleHistoryLimitBlur}
              className="w-[120px]"
            />
          </SettingContainer>
          <ToggleSwitch
            checked={settings?.assistant_auto_summarize ?? true}
            onChange={(value) =>
              setAndRefresh(commands.setAssistantAutoSummarize(value))
            }
            label={t("settings.assistant.autoSummarize.label")}
            description={t("settings.assistant.autoSummarize.description")}
            grouped={true}
          />
        </SettingsGroup>
      )}
    </div>
  );
};
