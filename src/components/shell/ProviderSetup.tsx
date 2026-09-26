import React, { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { toast } from "sonner";
import { ArrowUpRight, Loader2, RefreshCw } from "lucide-react";
import {
  commands,
  type AppSettings,
  type ModelChoice,
  type PostProcessProvider,
  type Result,
  type TtsVoice,
} from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { prettyModelName } from "@/lib/utils/prettyModelName";
import { ProviderTile } from "@/components/icons/ProviderLogos";
import { Dialog } from "@/components/ui/Dialog";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { LogoSelect } from "@/components/ui/LogoSelect";

/**
 * Setting up a provider from the picker that needs it.
 *
 * Picking a provider that has no key yet used to either do nothing useful or
 * send you to the Models page ("Browse all models") to find the right form.
 * Now the pickers list every provider, mark the ones that still need setup,
 * and choosing one opens this: the key (with where to get one), the address
 * when the provider needs one, and the model or voice — then "Save and use".
 */

/** Providers the backend refuses to call without a key. Mirrors
 *  `post_process_provider_requires_api_key` in src-tauri/src/settings.rs. */
export const LLM_REQUIRES_KEY = new Set([
  "openai",
  "zai",
  "openrouter",
  "anthropic",
  "groq",
  "cerebras",
  "gemini",
  "xai",
  "deepseek",
  "mistral",
  "moonshot",
  "together",
  "fireworks",
  "perplexity",
  "azure_openai",
  "bedrock_mantle",
]);

/** Where each provider hands out keys. Only pages we are sure of; a provider
 *  without one simply shows no link. */
const LLM_KEY_PAGES: Record<string, string> = {
  openai: "https://platform.openai.com/api-keys",
  anthropic: "https://console.anthropic.com/settings/keys",
  groq: "https://console.groq.com/keys",
  openrouter: "https://openrouter.ai/keys",
  gemini: "https://aistudio.google.com/apikey",
  xai: "https://console.x.ai",
  deepseek: "https://platform.deepseek.com/api_keys",
  mistral: "https://console.mistral.ai/api-keys",
  cerebras: "https://cloud.cerebras.ai",
  together: "https://api.together.ai",
  fireworks: "https://fireworks.ai",
  perplexity: "https://www.perplexity.ai/settings/api",
  moonshot: "https://platform.moonshot.ai",
  zai: "https://z.ai",
};

const TTS_KEY_PAGES: Record<string, string> = {
  openai: LLM_KEY_PAGES.openai,
  openrouter: LLM_KEY_PAGES.openrouter,
  elevenlabs: "https://elevenlabs.io/app/settings/api-keys",
};

/** Placeholders only: example values, never saved on their own. */
const URL_EXAMPLE = "https://my-server.example.com/v1";

/** Commands answer with a Result; a failure must not look like success. */
const settle = async <T,>(result: Promise<Result<T, string>>): Promise<T> => {
  const settled = await result;
  if (settled.status !== "ok") throw new Error(String(settled.error));
  return settled.data;
};

const errorText = (error: unknown): string =>
  error instanceof Error ? error.message : String(error);

const hostOf = (url: string): string => {
  try {
    return new URL(url).host.replace(/^www\./, "");
  } catch {
    return url;
  }
};

/** A model's name as a person would say it, unless the provider named it. */
const named = (id: string, label?: string | null): string =>
  label && label !== id ? label : prettyModelName(id) || id;

interface Choice {
  value: string;
  label: string;
}

/** Keep the value already chosen in the list, so it never reads as lost. */
const withCurrent = (options: Choice[], current: string): Choice[] => {
  const value = current.trim();
  if (!value || options.some((option) => option.value === value)) {
    return options;
  }
  return [{ value, label: named(value) }, ...options];
};

/** What was typed, as the id the provider expects: a typed display name is
 *  swapped for its id (OpenRouter rejects "Z.ai: GLM 5.3 Flash"). */
const resolveChoice = (input: string, options: Choice[]): string => {
  const wanted = input.trim();
  const hit =
    options.find((option) => option.value === wanted) ??
    options.find(
      (option) => option.label.toLowerCase() === wanted.toLowerCase(),
    );
  return hit ? hit.value : wanted;
};

/* ─────────────────────────────── pieces ─────────────────────────────── */

const Field: React.FC<{
  id?: string;
  label: string;
  note?: React.ReactNode;
  children: React.ReactNode;
}> = ({ id, label, note, children }) => (
  <div className="space-y-1.5">
    <label htmlFor={id} className="block text-[0.8125rem] font-medium text-ink">
      {label}
    </label>
    {children}
    {note && (
      <div className="flex min-h-4 items-start gap-3 text-xs leading-snug">
        {note}
      </div>
    )}
  </div>
);

const KeyLink: React.FC<{ url: string }> = ({ url }) => {
  const { t } = useTranslation();
  return (
    <button
      type="button"
      onClick={() => void openUrl(url).catch(() => {})}
      className="inline-flex cursor-pointer items-center gap-0.5 rounded font-medium text-muted transition-colors hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
    >
      {t("pickers.setup.getKey", { site: hostOf(url) })}
      <ArrowUpRight className="h-3 w-3" aria-hidden="true" />
    </button>
  );
};

/**
 * A model or voice: a searchable list once the provider has told us what it
 * offers, a text box before that (or when it offers no list), and a button
 * that asks again.
 */
const ChoiceField: React.FC<{
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  choices: Choice[];
  placeholder: string;
  loadLabel: string;
  loading: boolean;
  onLoad: () => void;
  error: string | null;
  autoFocus?: boolean;
}> = ({
  id,
  label,
  value,
  onChange,
  choices,
  placeholder,
  loadLabel,
  loading,
  onLoad,
  error,
  autoFocus = false,
}) => {
  const { t } = useTranslation();
  const [typing, setTyping] = useState(false);
  const listed = choices.length > 0 && !typing;
  const options = withCurrent(choices, value);

  return (
    <Field
      id={listed ? undefined : id}
      label={label}
      note={
        (error || choices.length > 0) && (
          <>
            <span className="min-w-0 flex-1 break-words text-error">
              {error}
            </span>
            {choices.length > 0 && (
              <button
                type="button"
                onClick={() => setTyping((current) => !current)}
                className="shrink-0 cursor-pointer rounded font-medium text-muted transition-colors hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
              >
                {typing
                  ? t("pickers.setup.pickFromList")
                  : t("pickers.setup.typeInstead")}
              </button>
            )}
          </>
        )
      }
    >
      <div className="flex gap-2">
        {listed ? (
          <LogoSelect
            options={options.map((option) => ({
              value: option.value,
              label: option.label,
              title: option.value,
            }))}
            value={value.trim() || null}
            onChange={onChange}
            placeholder={placeholder}
            ariaLabel={label}
            className="min-w-0 flex-1"
          />
        ) : (
          <Input
            id={id}
            value={value}
            onChange={(event) => onChange(event.target.value)}
            placeholder={placeholder}
            autoComplete="off"
            spellCheck={false}
            className="h-10 min-w-0 flex-1"
            data-autofocus={autoFocus ? "" : undefined}
          />
        )}
        <button
          type="button"
          onClick={onLoad}
          disabled={loading}
          title={loadLabel}
          aria-label={loadLabel}
          className="grid h-10 w-10 shrink-0 cursor-pointer place-items-center rounded-lg border border-hairline-strong bg-surface text-muted transition-colors hover:bg-surface-strong hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {loading ? (
            <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />
          ) : (
            <RefreshCw className="h-4 w-4" aria-hidden="true" />
          )}
        </button>
      </div>
    </Field>
  );
};

const SetupTitle: React.FC<{
  id: string;
  kind: "llm" | "tts";
  label: string;
}> = ({ id, kind, label }) => (
  <span className="flex items-center gap-3">
    <ProviderTile id={id} kind={kind} size="md" />
    <span className="min-w-0">{label}</span>
  </span>
);

const Actions: React.FC<{
  canSave: boolean;
  saving: boolean;
  onCancel: () => void;
}> = ({ canSave, saving, onCancel }) => {
  const { t } = useTranslation();
  return (
    <div className="flex items-center justify-end gap-2 pt-2">
      <Button type="button" variant="ghost" onClick={onCancel}>
        {t("common.cancel")}
      </Button>
      <Button type="submit" disabled={!canSave || saving}>
        {saving && (
          <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
        )}
        {t("pickers.setup.use")}
      </Button>
    </div>
  );
};

/* ─────────────────────────── language models ─────────────────────────── */

const LlmSetupForm: React.FC<{
  role: "assistant" | "cleanup";
  provider: PostProcessProvider;
  onDone: () => void;
}> = ({ role, provider, onDone }) => {
  const { t } = useTranslation();
  const { settings, refreshSettings, refreshPostProcessReadiness } =
    useSettings();
  const id = provider.id;
  const needsKey = LLM_REQUIRES_KEY.has(id);
  const showUrl = !!provider.allow_base_url_edit;
  const storedKey = settings?.post_process_api_keys?.[id]?.trim() ?? "";
  const storedUrl = provider.base_url?.trim() ?? "";
  const ownModels =
    role === "assistant"
      ? settings?.assistant_models
      : settings?.post_process_models;
  const otherModels =
    role === "assistant"
      ? settings?.post_process_models
      : settings?.assistant_models;

  const [key, setKey] = useState(storedKey);
  const [url, setUrl] = useState(storedUrl);
  // The other job's model for this provider is a good first guess.
  const [model, setModel] = useState(
    ownModels?.[id]?.trim() || otherModels?.[id]?.trim() || "",
  );
  const [models, setModels] = useState<ModelChoice[]>([]);
  const [loading, setLoading] = useState(false);
  const [listError, setListError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const saved = useRef({ key: storedKey, url: storedUrl });
  const keyId = useId();
  const urlId = useId();
  const modelId = useId();

  /** Write the address and key, so the provider can be asked for models. */
  const persistCredentials = async () => {
    let changed = false;
    const nextUrl = url.trim();
    if (showUrl && nextUrl && nextUrl !== saved.current.url) {
      await settle(commands.changePostProcessBaseUrlSetting(id, nextUrl));
      saved.current.url = nextUrl;
      changed = true;
    }
    const nextKey = key.trim();
    if (nextKey !== saved.current.key) {
      await settle(commands.changePostProcessApiKeySetting(id, nextKey));
      saved.current.key = nextKey;
      changed = true;
    }
    if (changed) {
      await refreshSettings();
      void refreshPostProcessReadiness();
    }
  };

  const load = async () => {
    setLoading(true);
    setListError(null);
    try {
      await persistCredentials();
      const list = await settle(commands.fetchPostProcessModels(id));
      setModels(list);
      if (list.length === 0) setListError(t("pickers.setup.noModels"));
    } catch (error) {
      setModels([]);
      setListError(errorText(error));
    } finally {
      setLoading(false);
    }
  };

  // Ask for the list as soon as there is something to ask with.
  useEffect(() => {
    if ((!needsKey || storedKey) && (!showUrl || storedUrl)) void load();
  }, []);

  const choices: Choice[] = models.map((item) => ({
    value: item.id,
    label: named(item.id, item.label),
  }));
  const canSave =
    !!model.trim() && (!needsKey || !!key.trim()) && (!showUrl || !!url.trim());

  const save = async () => {
    if (!canSave || saving) return;
    setSaving(true);
    try {
      await persistCredentials();
      const chosen = resolveChoice(model, choices);
      if (role === "assistant") {
        await settle(commands.changeAssistantModelSetting(id, chosen));
        await settle(commands.setAssistantProvider(id));
      } else {
        await settle(commands.changePostProcessModelSetting(id, chosen));
        await settle(commands.setPostProcessProvider(id));
      }
      await refreshSettings();
      void refreshPostProcessReadiness();
      toast.success(t("pickers.setup.done", { provider: provider.label }));
      onDone();
    } catch (error) {
      console.error(`Failed to set up ${id}:`, error);
      toast.error(errorText(error) || t("pickers.switchFailed"));
      await refreshSettings();
    } finally {
      setSaving(false);
    }
  };

  const keyPage = LLM_KEY_PAGES[id];
  // One field takes the cursor: the first one still empty.
  const focus: "url" | "key" | "model" =
    showUrl && !storedUrl ? "url" : !storedKey ? "key" : "model";

  return (
    <form
      className="space-y-4"
      onSubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      {showUrl && (
        <Field id={urlId} label={t("pickers.setup.endpoint")}>
          <Input
            id={urlId}
            value={url}
            onChange={(event) => setUrl(event.target.value)}
            onBlur={() => {
              if (url.trim() && url.trim() !== saved.current.url) void load();
            }}
            placeholder={URL_EXAMPLE}
            autoComplete="off"
            spellCheck={false}
            className="h-10 w-full"
            data-autofocus={focus === "url" ? "" : undefined}
          />
        </Field>
      )}
      <Field
        id={keyId}
        label={
          needsKey
            ? t("pickers.setup.apiKey")
            : t("pickers.setup.apiKeyOptional")
        }
        note={keyPage ? <KeyLink url={keyPage} /> : undefined}
      >
        <Input
          id={keyId}
          type="password"
          value={key}
          onChange={(event) => setKey(event.target.value)}
          onBlur={() => {
            // A pasted key is the moment the model list becomes available.
            if (key.trim() && key.trim() !== saved.current.key) void load();
          }}
          placeholder={t("pickers.setup.keyPlaceholder")}
          autoComplete="off"
          spellCheck={false}
          className="h-10 w-full"
          data-autofocus={focus === "key" ? "" : undefined}
        />
      </Field>
      <ChoiceField
        id={modelId}
        label={t("pickers.setup.model")}
        value={model}
        onChange={setModel}
        choices={choices}
        placeholder={t("pickers.setup.modelPlaceholder")}
        loadLabel={t("pickers.setup.loadModels")}
        loading={loading}
        onLoad={() => void load()}
        error={listError}
        autoFocus={focus === "model"}
      />
      <Actions canSave={canSave} saving={saving} onCancel={onDone} />
    </form>
  );
};

/**
 * "Set up Groq": the key, the address when the provider needs one, and the
 * model; saving also switches this job to it. Open with a provider id, close
 * with `null`.
 */
export const LlmProviderSetup: React.FC<{
  role: "assistant" | "cleanup";
  providerId: string | null;
  onClose: () => void;
}> = ({ role, providerId, onClose }) => {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const provider = providerId
    ? settings?.post_process_providers?.find((item) => item.id === providerId)
    : undefined;
  const needsKey = !!provider && LLM_REQUIRES_KEY.has(provider.id);
  const needsUrl = !!provider?.allow_base_url_edit;

  return (
    <Dialog
      open={!!provider}
      onClose={onClose}
      size="md"
      title={
        provider && (
          <SetupTitle
            id={provider.id}
            kind="llm"
            label={t("pickers.setup.title", { provider: provider.label })}
          />
        )
      }
      description={
        provider &&
        t(
          needsUrl && needsKey
            ? "pickers.setup.bodyBoth"
            : needsUrl
              ? "pickers.setup.bodyServer"
              : "pickers.setup.bodyKey",
        )
      }
    >
      {provider && (
        <LlmSetupForm
          key={provider.id}
          role={role}
          provider={provider}
          onDone={onClose}
        />
      )}
    </Dialog>
  );
};

/* ──────────────────────────────── voices ──────────────────────────────── */

interface TtsValues {
  url: string;
  key: string;
  model: string;
  voice: string;
}

const OPENAI_TTS_URL = "https://api.openai.com/v1";

/**
 * An engine's saved endpoint, key, model and voice. The engine in use keeps
 * its values in the flat fields (which older stores may hold alone); every
 * other engine keeps them in the per-engine maps.
 */
export const ttsValues = (
  settings: AppSettings | null | undefined,
  engine: string,
): TtsValues => {
  const active = settings?.assistant_tts_engine === engine;
  const pick = (
    flat: string | undefined,
    map: Partial<Record<string, string>> | undefined,
  ) => ((active ? flat : map?.[engine]) ?? "").trim();
  return {
    url:
      pick(
        settings?.assistant_tts_base_url,
        settings?.assistant_tts_base_urls,
      ) || (engine === "openai" ? OPENAI_TTS_URL : ""),
    key: pick(
      settings?.assistant_tts_api_key,
      settings?.assistant_tts_api_keys,
    ),
    model: pick(settings?.assistant_tts_model, settings?.assistant_tts_models),
    voice: pick(
      settings?.assistant_tts_remote_voice,
      settings?.assistant_tts_remote_voices,
    ),
  };
};

/** A self-hosted speech server legitimately needs no key. */
const isLoopback = (url: string) =>
  /^https?:\/\/(localhost|127\.0\.0\.1|\[::1\])(:|\/|$)/i.test(url.trim());

/** Whether an engine still needs something before it can speak. */
export const ttsNeedsSetup = (
  settings: AppSettings | null | undefined,
  engine: string,
): boolean => {
  if (engine === "kokoro") return false;
  const values = ttsValues(settings, engine);
  switch (engine) {
    case "openai":
      return !values.key && !isLoopback(values.url);
    case "elevenlabs":
      // It cannot speak without a voice id; there is no default voice.
      return !values.key || !values.voice;
    case "azure":
      return !values.key || !values.url;
    default:
      return !values.key;
  }
};

/** Which fields each remote engine has. Examples are placeholders only. */
const TTS_FIELDS: Record<
  string,
  {
    url?: { required: boolean; example: string };
    model?: { example: string };
    voice: { required: boolean; example?: string };
  }
> = {
  openai: {
    url: { required: false, example: OPENAI_TTS_URL },
    model: { example: "gpt-4o-mini-tts" },
    voice: { required: false, example: "alloy" },
  },
  openrouter: {
    model: { example: "gpt-4o-mini-tts" },
    voice: { required: false, example: "alloy" },
  },
  elevenlabs: {
    model: { example: "eleven_flash_v2_5" },
    voice: { required: true },
  },
  azure: {
    url: {
      required: true,
      example: "https://eastus2.tts.speech.microsoft.com",
    },
    voice: { required: false, example: "en-US-JennyNeural" },
  },
};

const VoiceSetupForm: React.FC<{
  engine: string;
  onSaved: () => void;
  onCancel: () => void;
}> = ({ engine, onSaved, onCancel }) => {
  const { t } = useTranslation();
  const { settings, refreshSettings } = useSettings();
  const fields = TTS_FIELDS[engine] ?? TTS_FIELDS.openai;
  const initial = useRef(ttsValues(settings, engine));
  const saved = useRef<TtsValues>({ ...initial.current });
  // The voice and model lists come from the engine in use, so setting one up
  // switches to it first. Cancelling switches back (see VoicePicker).
  const activeEngine = useRef(settings?.assistant_tts_engine ?? "kokoro");

  const [url, setUrl] = useState(initial.current.url);
  const [key, setKey] = useState(initial.current.key);
  const [model, setModel] = useState(initial.current.model);
  const [voice, setVoice] = useState(initial.current.voice);
  const [voices, setVoices] = useState<TtsVoice[]>([]);
  const [models, setModels] = useState<string[]>([]);
  const [loadingVoices, setLoadingVoices] = useState(false);
  const [loadingModels, setLoadingModels] = useState(false);
  const [voiceError, setVoiceError] = useState<string | null>(null);
  const [modelError, setModelError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const urlId = useId();
  const keyId = useId();
  const voiceId = useId();
  const modelId = useId();

  const persist = async (values: TtsValues = { url, key, model, voice }) => {
    let changed = false;
    if (activeEngine.current !== engine) {
      await settle(commands.setAssistantTtsEngine(engine));
      activeEngine.current = engine;
      changed = true;
    }
    const put = async (
      field: keyof TtsValues,
      write: (value: string) => Promise<Result<null, string>>,
    ) => {
      const next = values[field].trim();
      if (next === saved.current[field]) return;
      await settle(write(next));
      saved.current[field] = next;
      changed = true;
    };
    if (fields.url) await put("url", commands.setAssistantTtsBaseUrl);
    await put("key", commands.setAssistantTtsApiKey);
    if (fields.model) await put("model", commands.setAssistantTtsModel);
    await put("voice", commands.setAssistantTtsRemoteVoice);
    if (changed) await refreshSettings();
  };

  const loadVoices = async () => {
    setLoadingVoices(true);
    setVoiceError(null);
    try {
      await persist();
      const list = await settle(commands.assistantListTtsVoices());
      setVoices(list);
      if (list.length === 0) setVoiceError(t("pickers.voiceSetup.noVoices"));
    } catch (error) {
      setVoices([]);
      setVoiceError(errorText(error));
    } finally {
      setLoadingVoices(false);
    }
  };

  const loadModels = async () => {
    setLoadingModels(true);
    setModelError(null);
    try {
      await persist();
      const list = await settle(commands.assistantListTtsModels());
      setModels(list);
      if (list.length === 0) setModelError(t("pickers.setup.noModels"));
    } catch (error) {
      setModels([]);
      setModelError(errorText(error));
    } finally {
      setLoadingModels(false);
    }
  };

  // With a key already saved, the voices can be listed straight away.
  useEffect(() => {
    const ready =
      !!initial.current.key && (!fields.url?.required || !!initial.current.url);
    if (ready) void loadVoices();
  }, []);

  const keyOptional = engine === "openai" && isLoopback(url);
  const canSave =
    (keyOptional || !!key.trim()) &&
    (!fields.url?.required || !!url.trim()) &&
    (!fields.voice.required || !!voice.trim());

  const voiceChoices: Choice[] = voices.map((item) => ({
    value: item.id,
    label: item.label || item.id,
  }));
  const modelChoices: Choice[] = models.map((item) => ({
    value: item,
    label: named(item),
  }));

  const save = async () => {
    if (!canSave || saving) return;
    setSaving(true);
    try {
      const chosenVoice = resolveChoice(voice, voiceChoices);
      const chosenModel = resolveChoice(model, modelChoices);
      setVoice(chosenVoice);
      setModel(chosenModel);
      await persist({ url, key, model: chosenModel, voice: chosenVoice });
      const name = t(`voiceEngines.names.${engine}`);
      toast.success(t("pickers.setup.done", { provider: name }));
      onSaved();
    } catch (error) {
      console.error(`Failed to set up ${engine}:`, error);
      toast.error(errorText(error) || t("pickers.switchFailed"));
    } finally {
      setSaving(false);
    }
  };

  const keyPage = TTS_KEY_PAGES[engine];
  const focus: "url" | "key" | "voice" =
    fields.url?.required && !initial.current.url
      ? "url"
      : !initial.current.key
        ? "key"
        : "voice";

  return (
    <form
      className="space-y-4"
      onSubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      {fields.url && (
        <Field id={urlId} label={t("pickers.setup.endpoint")}>
          <Input
            id={urlId}
            value={url}
            onChange={(event) => setUrl(event.target.value)}
            placeholder={fields.url.example}
            autoComplete="off"
            spellCheck={false}
            className="h-10 w-full"
            data-autofocus={focus === "url" ? "" : undefined}
          />
        </Field>
      )}
      <Field
        id={keyId}
        label={
          keyOptional
            ? t("pickers.setup.apiKeyOptional")
            : t("pickers.setup.apiKey")
        }
        note={keyPage ? <KeyLink url={keyPage} /> : undefined}
      >
        <Input
          id={keyId}
          type="password"
          value={key}
          onChange={(event) => setKey(event.target.value)}
          onBlur={() => {
            // A pasted key is the moment the voices can be listed.
            if (key.trim() && key.trim() !== saved.current.key) {
              void loadVoices();
            }
          }}
          placeholder={t("pickers.setup.keyPlaceholder")}
          autoComplete="off"
          spellCheck={false}
          className="h-10 w-full"
          data-autofocus={focus === "key" ? "" : undefined}
        />
      </Field>
      <ChoiceField
        id={voiceId}
        label={t("pickers.voiceSetup.voice")}
        value={voice}
        onChange={setVoice}
        choices={voiceChoices}
        placeholder={
          fields.voice.example ?? t("pickers.voiceSetup.voicePlaceholder")
        }
        loadLabel={t("pickers.voiceSetup.loadVoices")}
        loading={loadingVoices}
        onLoad={() => void loadVoices()}
        error={voiceError}
        autoFocus={focus === "voice"}
      />
      {fields.model && (
        <ChoiceField
          id={modelId}
          label={t("pickers.voiceSetup.model")}
          value={model}
          onChange={setModel}
          choices={modelChoices}
          placeholder={fields.model.example}
          loadLabel={t("pickers.setup.loadModels")}
          loading={loadingModels}
          onLoad={() => void loadModels()}
          error={modelError}
        />
      )}
      <Actions canSave={canSave} saving={saving} onCancel={onCancel} />
    </form>
  );
};

/**
 * "Set up ElevenLabs": the key, the endpoint for engines that need one, and a
 * voice. `onCancel` is also what Escape and the close button call, so the
 * caller can put back the engine that was in use.
 */
export const VoiceEngineSetup: React.FC<{
  engine: string | null;
  onSaved: () => void;
  onCancel: () => void;
}> = ({ engine, onSaved, onCancel }) => {
  const { t } = useTranslation();
  const name = engine ? t(`voiceEngines.names.${engine}`) : "";
  return (
    <Dialog
      open={!!engine}
      onClose={onCancel}
      size="md"
      title={
        engine && (
          <SetupTitle
            id={engine}
            kind="tts"
            label={t("pickers.setup.title", { provider: name })}
          />
        )
      }
      description={
        engine &&
        (TTS_FIELDS[engine]?.url?.required
          ? t("pickers.voiceSetup.bodyEndpoint")
          : t("pickers.voiceSetup.bodyKey"))
      }
    >
      {engine && (
        <VoiceSetupForm
          key={engine}
          engine={engine}
          onSaved={onSaved}
          onCancel={onCancel}
        />
      )}
    </Dialog>
  );
};
