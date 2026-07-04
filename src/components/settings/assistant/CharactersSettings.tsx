import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { open, save } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import {
  Copy,
  Download,
  ImagePlus,
  Loader2,
  Mic,
  Plus,
  RotateCcw,
  Square,
  Trash2,
  Upload,
  Wand2,
  X,
} from "lucide-react";
import {
  commands,
  type AssistantCharacter,
  type AssistantResponseLength,
} from "@/bindings";
import { Button } from "@/components/ui/Button";
import { Input } from "../../ui/Input";
import { Textarea } from "@/components/ui";
import { Dropdown } from "../../ui/Dropdown";
import { useSettings } from "../../../hooks/useSettings";

/** A stable-ish unique id for a new/imported/duplicated character. The backend
 *  also enforces uniqueness, so a collision is only cosmetic. */
const newId = (): string =>
  `char-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;

const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];

/** Round avatar: the uploaded image, a cat emoji for the Cat, or the name's
 *  first initial as a fallback. */
const Avatar: React.FC<{
  character: AssistantCharacter | null;
  size: number;
}> = ({ character, size }) => {
  if (character?.avatar) {
    return (
      <img
        src={character.avatar}
        alt=""
        className="rounded-full object-cover shrink-0"
        style={{ width: size, height: size }}
      />
    );
  }
  const fallback =
    character?.kind === "cat"
      ? "🐱"
      : (character?.name.trim()[0] ?? "?").toUpperCase();
  return (
    <span
      className="rounded-full bg-hairline-strong/60 text-body flex items-center justify-center shrink-0 select-none font-semibold"
      style={{ width: size, height: size, fontSize: Math.round(size * 0.42) }}
      aria-hidden
    >
      {fallback}
    </span>
  );
};

/** Small uppercase section label, matching the SettingsGroup heading style. */
const SectionLabel: React.FC<{ children: React.ReactNode }> = ({
  children,
}) => (
  <h2 className="px-1 text-xs font-semibold uppercase tracking-[0.08em] text-muted">
    {children}
  </h2>
);

/** A labelled form field (label above the control). */
const Field: React.FC<{ label: string; children: React.ReactNode }> = ({
  label,
  children,
}) => (
  <label className="block space-y-1.5">
    <span className="block text-[11px] font-medium uppercase tracking-wide text-muted">
      {label}
    </span>
    {children}
  </label>
);

/**
 * Assistant "characters" (personas) — its own settings section. Pick the active
 * one from a gallery, edit its name / avatar / persona prompt / greeting, create
 * a blank one, generate one with the LLM from a description, duplicate,
 * import/export as JSON, or delete. The active character's prompt overrides the
 * plain system prompt for LLM turns; the "Cat" character ignores the model
 * entirely and just meows.
 */
export const CharactersSettings: React.FC = () => {
  const { t } = useTranslation();
  const { settings, refreshSettings } = useSettings();

  const characters = settings?.assistant_characters ?? [];
  const activeId = settings?.assistant_active_character_id ?? "default";
  const selected = characters.find((c) => c.id === activeId) ?? characters[0];

  const [draftName, setDraftName] = useState("");
  const [draftDescription, setDraftDescription] = useState("");
  const [draftPrompt, setDraftPrompt] = useState("");
  const [draftGreeting, setDraftGreeting] = useState("");
  const [error, setError] = useState<string | null>(null);

  const [showAi, setShowAi] = useState(false);
  const [aiDesc, setAiDesc] = useState("");
  const [aiLoading, setAiLoading] = useState(false);
  const [aiError, setAiError] = useState<string | null>(null);
  // In-app dictation for the "Describe your persona" box. The transcript is
  // delivered to this webview via the `dictation-transcript` event (see the
  // listener effect below) rather than pasted into the focused OS field, so no
  // focus juggling is needed — `dictating` just drives the mic button's state.
  const aiBoxRef = useRef<HTMLDivElement>(null);
  const [dictating, setDictating] = useState(false);

  // Reseed the editable drafts whenever the selected character changes.
  useEffect(() => {
    setDraftName(selected?.name ?? "");
    setDraftDescription(selected?.description ?? "");
    setDraftPrompt(selected?.prompt ?? "");
    setDraftGreeting(selected?.greeting ?? "");
  }, [selected?.id]);

  // In-app dictation delivers its transcript here as an event (see the
  // `toggle_dictation` command and `DICTATE_TO_FIELD` in the backend) rather
  // than pasting into the focused OS window, which is unreliable for a webview
  // textarea. Subscribe only while the Create-with-AI box is open, and append
  // whatever comes back to the description.
  useEffect(() => {
    if (!showAi) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void listen<string>("dictation-transcript", (event) => {
      const text = (event.payload ?? "").trim();
      setDictating(false);
      if (!text) return;
      setAiDesc((prev) => (prev.trim() ? `${prev.trimEnd()} ${text}` : text));
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [showAi]);

  const saveCharacters = useCallback(
    async (next: AssistantCharacter[]): Promise<boolean> => {
      const res = await commands.setAssistantCharacters(next);
      if (res.status === "error") {
        setError(res.error);
        return false;
      }
      setError(null);
      await refreshSettings();
      return true;
    },
    [refreshSettings],
  );

  const activate = useCallback(
    async (id: string) => {
      const res = await commands.setAssistantActiveCharacter(id);
      if (res.status === "error") {
        setError(res.error);
        return;
      }
      await refreshSettings();
    },
    [refreshSettings],
  );

  const patchSelected = useCallback(
    async (patch: Partial<AssistantCharacter>) => {
      if (!selected) return;
      const next = characters.map((c) =>
        c.id === selected.id ? { ...c, ...patch } : c,
      );
      await saveCharacters(next);
    },
    [characters, selected, saveCharacters],
  );

  const createBlank = useCallback(async () => {
    const id = newId();
    const character: AssistantCharacter = {
      id,
      name: t("settings.assistant.characters.newName"),
      prompt: "",
      greeting: "",
      avatar: "",
      kind: "llm",
      builtin: false,
      description: "",
      response_length: null,
    };
    if (await saveCharacters([...characters, character])) {
      await activate(id);
    }
  }, [characters, saveCharacters, activate, t]);

  const duplicate = useCallback(async () => {
    if (!selected) return;
    const id = newId();
    const copy: AssistantCharacter = {
      ...selected,
      id,
      name: t("settings.assistant.characters.copyName", {
        name: selected.name,
      }),
      builtin: false,
    };
    const index = characters.findIndex((c) => c.id === selected.id);
    const next = [...characters];
    next.splice(index + 1, 0, copy);
    if (await saveCharacters(next)) {
      await activate(id);
    }
  }, [characters, selected, saveCharacters, activate, t]);

  const remove = useCallback(async () => {
    if (!selected || selected.id === "default") return;
    const next = characters.filter((c) => c.id !== selected.id);
    if (await saveCharacters(next)) {
      await activate("default");
    }
  }, [characters, selected, saveCharacters, activate]);

  // Reset an edited built-in persona back to the version shipped with the app.
  const restoreDefault = useCallback(async () => {
    if (!selected?.builtin) return;
    const res = await commands.assistantRestoreBuiltinCharacter(selected.id);
    if (res.status === "error") {
      setError(res.error);
      return;
    }
    setError(null);
    // Sync the editor fields immediately (the selected id is unchanged, so the
    // draft-reseeding effect won't fire on its own).
    setDraftName(res.data.name);
    setDraftDescription(res.data.description ?? "");
    setDraftPrompt(res.data.prompt ?? "");
    setDraftGreeting(res.data.greeting ?? "");
    await refreshSettings();
  }, [selected, refreshSettings]);

  // Re-add any built-in personas that were deleted (leaves customs untouched).
  const restoreMissing = useCallback(async () => {
    const res = await commands.assistantRestoreMissingBuiltins();
    if (res.status === "error") {
      setError(res.error);
      return;
    }
    setError(null);
    await refreshSettings();
  }, [refreshSettings]);

  const uploadAvatar = useCallback(async () => {
    try {
      const path = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "Image", extensions: IMAGE_EXTENSIONS }],
      });
      if (typeof path !== "string") return;
      const res = await commands.assistantReadAvatar(path);
      if (res.status === "error") {
        setError(res.error);
        return;
      }
      await patchSelected({ avatar: res.data });
    } catch (err) {
      setError(String(err));
    }
  }, [patchSelected]);

  const importCharacter = useCallback(async () => {
    try {
      const path = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "Persona", extensions: ["json"] }],
      });
      if (typeof path !== "string") return;
      const res = await commands.assistantImportCharacter(path);
      if (res.status === "error") {
        setError(res.error);
        return;
      }
      await refreshSettings();
      await activate(res.data.id);
    } catch (err) {
      setError(String(err));
    }
  }, [refreshSettings, activate]);

  const exportCharacter = useCallback(async () => {
    if (!selected) return;
    try {
      const path = await save({
        defaultPath: `${selected.name || "persona"}.json`,
        filters: [{ name: "Persona", extensions: ["json"] }],
      });
      if (!path) return;
      const res = await commands.assistantExportCharacter(selected.id, path);
      if (res.status === "error") setError(res.error);
    } catch (err) {
      setError(String(err));
    }
  }, [selected]);

  const generate = useCallback(async () => {
    const description = aiDesc.trim();
    if (!description) return;
    setAiLoading(true);
    setAiError(null);
    try {
      const res = await commands.assistantGenerateCharacter(description);
      if (res.status === "error") {
        setAiError(res.error);
        return;
      }
      const id = newId();
      const character: AssistantCharacter = {
        id,
        name: res.data.name,
        prompt: res.data.prompt,
        greeting: res.data.greeting,
        avatar: "",
        kind: "llm",
        builtin: false,
        description: "",
        response_length: null,
      };
      if (await saveCharacters([...characters, character])) {
        await activate(id);
        setShowAi(false);
        setAiDesc("");
        setDictating(false);
      }
    } catch (err) {
      setAiError(String(err));
    } finally {
      setAiLoading(false);
    }
  }, [aiDesc, characters, saveCharacters, activate]);

  // Toggle in-app dictation for the description box. First tap starts a
  // hands-free recording, second tap stops it; the transcript comes back
  // through the `dictation-transcript` event and is appended to the box.
  const toggleDictation = useCallback(async () => {
    setDictating((d) => !d);
    try {
      await commands.toggleDictation();
    } catch {
      setDictating(false);
    }
  }, []);

  // Close the "Create with AI" box, cancelling any in-progress dictation so we
  // never leave a recording running for a hidden field.
  const closeAi = useCallback(() => {
    if (dictating) {
      setDictating(false);
      void commands.cancelOperation().catch(() => {});
    }
    setShowAi(false);
  }, [dictating]);

  if (!settings) return null;

  const isCat = selected?.kind === "cat";

  const subtitle = (c: AssistantCharacter): string => {
    if (c.kind === "cat") return t("settings.assistant.characters.meowsOnly");
    const description = c.description?.trim();
    if (description) return description;
    return c.builtin
      ? t("settings.assistant.characters.builtin")
      : t("settings.assistant.characters.custom");
  };

  return (
    <div className="max-w-2xl w-full mx-auto space-y-8">
      {/* Gallery ---------------------------------------------------------- */}
      <section className="space-y-3">
        <div className="px-1">
          <SectionLabel>
            {t("settings.assistant.characters.galleryLabel")}
          </SectionLabel>
          <p className="mt-1 text-xs text-muted leading-relaxed max-w-lg">
            {t("settings.assistant.characters.description")}
          </p>
        </div>

        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2.5">
          {characters.map((character) => {
            const active = character.id === activeId;
            return (
              <button
                key={character.id}
                type="button"
                onClick={() => activate(character.id)}
                className={`flex items-center gap-3 rounded-xl border p-3 text-start transition-colors cursor-pointer ${
                  active
                    ? "border-accent/60 bg-accent/5 ring-1 ring-accent/25"
                    : "border-hairline bg-surface hover:bg-surface-strong hover:border-hairline-strong"
                }`}
              >
                <Avatar character={character} size={40} />
                <div className="min-w-0 flex-1">
                  <span className="block text-[13px] font-medium text-ink truncate">
                    {character.name}
                  </span>
                  <span className="block text-[11.5px] text-muted truncate">
                    {subtitle(character)}
                  </span>
                </div>
                {active && (
                  <span className="shrink-0 rounded-full bg-accent/10 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-accent">
                    {t("settings.assistant.characters.active")}
                  </span>
                )}
              </button>
            );
          })}
        </div>

        <div className="flex flex-wrap gap-2 px-0.5">
          <Button variant="secondary" size="sm" onClick={createBlank}>
            <Plus size={14} />
            {t("settings.assistant.characters.new")}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => (showAi ? closeAi() : setShowAi(true))}
          >
            <Wand2 size={14} />
            {t("settings.assistant.characters.createAi")}
          </Button>
          <Button variant="secondary" size="sm" onClick={importCharacter}>
            <Upload size={14} />
            {t("settings.assistant.characters.import")}
          </Button>
          <Button variant="secondary" size="sm" onClick={restoreMissing}>
            <RotateCcw size={14} />
            {t("settings.assistant.characters.restoreBuiltins")}
          </Button>
        </div>

        {showAi && (
          <div className="rounded-xl border border-hairline bg-surface p-4 space-y-3">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <Wand2 size={15} className="text-accent" />
                <span className="text-[13px] font-medium text-ink">
                  {t("settings.assistant.characters.aiTitle")}
                </span>
              </div>
              <button
                type="button"
                onClick={closeAi}
                title={t("settings.assistant.characters.aiClose")}
                className="text-muted hover:text-ink transition-colors cursor-pointer"
              >
                <X size={15} />
              </button>
            </div>
            <div className="relative" ref={aiBoxRef}>
              <Textarea
                value={aiDesc}
                onChange={(e) => setAiDesc(e.target.value)}
                placeholder={t("settings.assistant.characters.aiPlaceholder")}
                rows={3}
                className="w-full pr-11"
              />
              <button
                type="button"
                // Keep the textarea's caret/selection intact on click; the
                // transcript arrives via the dictation-transcript event.
                onMouseDown={(e) => e.preventDefault()}
                onClick={toggleDictation}
                title={t(
                  dictating
                    ? "settings.assistant.characters.aiDictateStop"
                    : "settings.assistant.characters.aiDictate",
                )}
                aria-label={t(
                  dictating
                    ? "settings.assistant.characters.aiDictateStop"
                    : "settings.assistant.characters.aiDictate",
                )}
                aria-pressed={dictating}
                className={`absolute right-2 top-2 flex h-7 w-7 items-center justify-center rounded-lg border transition-colors cursor-pointer ${
                  dictating
                    ? "border-transparent bg-accent text-white animate-pulse"
                    : "border-hairline-strong bg-surface text-muted hover:text-ink hover:border-ink/40"
                }`}
              >
                {dictating ? <Square size={13} /> : <Mic size={14} />}
              </button>
            </div>
            <div className="flex items-center gap-3">
              <Button
                variant="primary"
                size="sm"
                disabled={aiLoading || !aiDesc.trim()}
                onClick={generate}
              >
                {aiLoading ? (
                  <Loader2 size={14} className="animate-spin" />
                ) : (
                  <Wand2 size={14} />
                )}
                {t("settings.assistant.characters.generate")}
              </Button>
              {aiError && (
                <span className="text-xs text-error leading-snug">
                  {aiError}
                </span>
              )}
            </div>
          </div>
        )}
      </section>

      {/* Editor ----------------------------------------------------------- */}
      {selected && (
        <section className="space-y-3">
          <SectionLabel>
            {t("settings.assistant.characters.editSection")}
          </SectionLabel>

          <div className="rounded-xl border border-hairline bg-surface p-5 space-y-5">
            <div className="flex items-center gap-4">
              <Avatar character={selected} size={60} />
              <div className="space-y-2">
                <span className="block text-[11px] font-medium uppercase tracking-wide text-muted">
                  {t("settings.assistant.characters.avatarLabel")}
                </span>
                <div className="flex flex-wrap gap-2">
                  <Button variant="secondary" size="sm" onClick={uploadAvatar}>
                    <ImagePlus size={14} />
                    {t("settings.assistant.characters.avatarUpload")}
                  </Button>
                  {selected.avatar && (
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => patchSelected({ avatar: "" })}
                    >
                      {t("settings.assistant.characters.avatarRemove")}
                    </Button>
                  )}
                </div>
              </div>
            </div>

            <Field label={t("settings.assistant.characters.nameLabel")}>
              <Input
                type="text"
                value={draftName}
                onChange={(e) => setDraftName(e.target.value)}
                onBlur={() => {
                  const name = draftName.trim();
                  if (name && name !== selected.name) patchSelected({ name });
                  else setDraftName(selected.name);
                }}
                className="w-full"
              />
            </Field>

            <Field label={t("settings.assistant.characters.roleLabel")}>
              <Input
                type="text"
                value={draftDescription}
                onChange={(e) => setDraftDescription(e.target.value)}
                onBlur={() => {
                  const description = draftDescription.trim();
                  if (description !== (selected.description ?? ""))
                    patchSelected({ description });
                }}
                placeholder={t("settings.assistant.characters.rolePlaceholder")}
                className="w-full"
              />
            </Field>

            {isCat ? (
              <div className="rounded-lg border border-hairline bg-surface-strong/40 px-3.5 py-3">
                <p className="text-xs text-muted leading-relaxed">
                  {t("settings.assistant.characters.catNote")}
                </p>
              </div>
            ) : (
              <Field label={t("settings.assistant.characters.promptLabel")}>
                <Textarea
                  value={draftPrompt}
                  onChange={(e) => setDraftPrompt(e.target.value)}
                  onBlur={() => {
                    if (draftPrompt !== selected.prompt)
                      patchSelected({ prompt: draftPrompt });
                  }}
                  rows={6}
                  className="w-full"
                />
              </Field>
            )}

            {!isCat && (
              <div className="space-y-1.5">
                <span className="block text-[11px] font-medium uppercase tracking-wide text-muted">
                  {t("settings.assistant.characters.responseLength.label")}
                </span>
                <Dropdown
                  options={[
                    {
                      value: "inherit",
                      label: t(
                        "settings.assistant.characters.responseLength.options.inherit",
                      ),
                    },
                    {
                      value: "short",
                      label: t(
                        "settings.assistant.characters.responseLength.options.short",
                      ),
                    },
                    {
                      value: "medium",
                      label: t(
                        "settings.assistant.characters.responseLength.options.medium",
                      ),
                    },
                    {
                      value: "long",
                      label: t(
                        "settings.assistant.characters.responseLength.options.long",
                      ),
                    },
                  ]}
                  selectedValue={selected.response_length ?? "inherit"}
                  onSelect={(value) =>
                    patchSelected({
                      response_length:
                        value === "inherit"
                          ? null
                          : (value as AssistantResponseLength),
                    })
                  }
                />
                <p className="text-[11px] text-muted leading-relaxed">
                  {t("settings.assistant.characters.responseLength.hint")}
                </p>
              </div>
            )}

            <div className="border-t border-hairline pt-4">
              <Field label={t("settings.assistant.characters.greetingLabel")}>
                <Input
                  type="text"
                  value={draftGreeting}
                  onChange={(e) => setDraftGreeting(e.target.value)}
                  onBlur={() => {
                    if (draftGreeting !== selected.greeting)
                      patchSelected({ greeting: draftGreeting });
                  }}
                  placeholder={t(
                    "settings.assistant.characters.greetingPlaceholder",
                  )}
                  className="w-full"
                />
              </Field>
            </div>

            <div className="flex items-center gap-2 border-t border-hairline pt-4">
              <Button variant="secondary" size="sm" onClick={duplicate}>
                <Copy size={14} />
                {t("settings.assistant.characters.duplicate")}
              </Button>
              <Button variant="secondary" size="sm" onClick={exportCharacter}>
                <Download size={14} />
                {t("settings.assistant.characters.export")}
              </Button>
              {selected.builtin && (
                <Button variant="secondary" size="sm" onClick={restoreDefault}>
                  <RotateCcw size={14} />
                  {t("settings.assistant.characters.restoreDefault")}
                </Button>
              )}
              {selected.id !== "default" && (
                <Button
                  variant="danger-ghost"
                  size="sm"
                  className="ml-auto"
                  onClick={remove}
                >
                  <Trash2 size={14} />
                  {t("settings.assistant.characters.delete")}
                </Button>
              )}
            </div>

            {error && <p className="text-xs text-error">{error}</p>}
          </div>
        </section>
      )}
    </div>
  );
};

export default CharactersSettings;
