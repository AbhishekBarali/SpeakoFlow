import React, { useCallback, useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { open, save } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import {
  Check,
  Loader2,
  Mic,
  MoreHorizontal,
  Plus,
  Square,
} from "lucide-react";
import {
  commands,
  type AssistantCharacter,
  type AssistantResponseLength,
} from "@/bindings";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Textarea } from "@/components/ui/Textarea";
import { Segmented } from "@/components/ui/Segmented";
import { InfoTip } from "@/components/ui/InfoTip";
import { Dialog } from "@/components/ui/Dialog";
import { MenuButton, type MenuItem } from "@/components/ui/Menu";
import { useSettings } from "@/hooks/useSettings";

/** A unique-enough id for a new, imported, or duplicated profile. The backend
 *  also enforces uniqueness, so a collision is only cosmetic. */
const newId = (): string =>
  `char-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;

const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];

/**
 * A profile's picture: the image the user gave it, or its initial on a quiet
 * disc. Every profile gets the same disc — the old gradient circles with a
 * glyph each (a heart, a flame, a cat) read as decoration, not identity.
 */
export const Avatar: React.FC<{
  character: AssistantCharacter | null;
  size: number;
}> = ({ character, size }) => {
  if (character?.avatar) {
    return (
      <img
        src={character.avatar}
        alt=""
        className="shrink-0 rounded-full object-cover"
        style={{ width: size, height: size }}
      />
    );
  }
  const initial = (character?.name.trim()[0] ?? "?").toUpperCase();
  return (
    <span
      aria-hidden="true"
      className="grid shrink-0 select-none place-items-center rounded-full bg-ink/[0.07] font-semibold text-body"
      style={{ width: size, height: size, fontSize: Math.round(size * 0.4) }}
    >
      {initial}
    </span>
  );
};

type LengthChoice = "inherit" | Exclude<AssistantResponseLength, "default">;
const LENGTHS: LengthChoice[] = ["inherit", "short", "medium", "long"];

/** A label (with an optional short (i)) above its field. */
const Field: React.FC<{
  label: string;
  tip?: string;
  htmlFor?: string;
  children: React.ReactNode;
}> = ({ label, tip, htmlFor, children }) => (
  <div className="space-y-1.5">
    <div className="flex items-center gap-1">
      <label
        htmlFor={htmlFor}
        className="text-[0.8125rem] font-medium text-ink"
      >
        {label}
      </label>
      {tip && <InfoTip text={tip} />}
    </div>
    {children}
  </div>
);

const ICON_BUTTON =
  "grid h-8 w-8 shrink-0 cursor-pointer place-items-center rounded-lg text-muted transition-colors hover:bg-ink/[0.05] hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40";

/**
 * Profiles: every persona in a list on the left, the one you pick editable on
 * the right.
 *
 * Picking a profile in the list only opens it; "Use this profile" is what
 * switches the assistant to it. The old screen activated whatever you clicked,
 * so looking at a profile changed the assistant, and editing one meant using
 * it first. The one in use carries a check in the list.
 */
export const CharactersSettings: React.FC = () => {
  const { t } = useTranslation();
  const { settings, refreshSettings } = useSettings();

  const characters = settings?.assistant_characters ?? [];
  const activeId = settings?.assistant_active_character_id ?? "default";
  const [editingId, setEditingId] = useState<string>(activeId);
  const [mode, setMode] = useState<"edit" | "ai">("edit");
  const selected =
    characters.find((c) => c.id === editingId) ??
    characters.find((c) => c.id === activeId) ??
    characters[0];

  const [draftName, setDraftName] = useState("");
  const [draftDescription, setDraftDescription] = useState("");
  const [draftPrompt, setDraftPrompt] = useState("");
  const [draftGreeting, setDraftGreeting] = useState("");
  const [confirmDelete, setConfirmDelete] = useState(false);
  const focusNameNext = useRef(false);
  const nameRef = useRef<HTMLInputElement>(null);

  const [aiDesc, setAiDesc] = useState("");
  const [aiLoading, setAiLoading] = useState(false);
  const [aiError, setAiError] = useState<string | null>(null);
  const [dictating, setDictating] = useState(false);

  const nameId = useId();
  const roleId = useId();
  const promptId = useId();
  const greetingId = useId();
  const aiId = useId();

  // Reseed the fields whenever a different profile is opened.
  useEffect(() => {
    setDraftName(selected?.name ?? "");
    setDraftDescription(selected?.description ?? "");
    setDraftPrompt(selected?.prompt ?? "");
    setDraftGreeting(selected?.greeting ?? "");
    setConfirmDelete(false);
    if (focusNameNext.current) {
      focusNameNext.current = false;
      window.requestAnimationFrame(() => nameRef.current?.select());
    }
    // Keyed on the id on purpose: a save of this profile must not reset a
    // field the user is still typing in.
  }, [selected?.id]);

  // In-app dictation delivers its transcript here as an event (see
  // `toggle_dictation` and `DICTATE_TO_FIELD` in the backend) rather than
  // pasting into the focused OS window, which is unreliable for a webview
  // field. Listen only while the describe-it pane is open.
  useEffect(() => {
    if (mode !== "ai") return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void listen<string>("dictation-transcript", (event) => {
      const text = (event.payload ?? "").trim();
      setDictating(false);
      if (!text) return;
      setAiDesc((prev) => (prev.trim() ? `${prev.trimEnd()} ${text}` : text));
    }).then((off) => {
      if (cancelled) off();
      else unlisten = off;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [mode]);

  const fail = useCallback(
    (error: unknown) => {
      console.error("Profile action failed:", error);
      toast.error(
        typeof error === "string" && error ? error : t("common.saveFailed"),
      );
    },
    [t],
  );

  const saveCharacters = useCallback(
    async (next: AssistantCharacter[]): Promise<boolean> => {
      const result = await commands.setAssistantCharacters(next);
      await refreshSettings();
      if (result.status === "error") {
        fail(result.error);
        return false;
      }
      return true;
    },
    [refreshSettings, fail],
  );

  const activate = useCallback(
    async (id: string) => {
      const result = await commands.setAssistantActiveCharacter(id);
      await refreshSettings();
      if (result.status === "error") fail(result.error);
    },
    [refreshSettings, fail],
  );

  const patchSelected = useCallback(
    async (patch: Partial<AssistantCharacter>) => {
      if (!selected) return;
      await saveCharacters(
        characters.map((c) => (c.id === selected.id ? { ...c, ...patch } : c)),
      );
    },
    [characters, selected, saveCharacters],
  );

  const openProfile = (id: string) => {
    setMode("edit");
    setEditingId(id);
  };

  const createBlank = async () => {
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
      focusNameNext.current = true;
      openProfile(id);
    }
  };

  const duplicate = async () => {
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
    if (await saveCharacters(next)) openProfile(id);
  };

  const remove = async () => {
    if (!selected || selected.id === "default") return;
    const wasActive = selected.id === activeId;
    if (await saveCharacters(characters.filter((c) => c.id !== selected.id))) {
      if (wasActive) await activate("default");
      openProfile(wasActive ? "default" : activeId);
    }
    setConfirmDelete(false);
  };

  const restoreDefault = async () => {
    if (!selected?.builtin) return;
    const result = await commands.assistantRestoreBuiltinCharacter(selected.id);
    if (result.status === "error") {
      fail(result.error);
      return;
    }
    // Same id, so the reseed effect will not run on its own.
    setDraftName(result.data.name);
    setDraftDescription(result.data.description ?? "");
    setDraftPrompt(result.data.prompt ?? "");
    setDraftGreeting(result.data.greeting ?? "");
    await refreshSettings();
  };

  const restoreMissing = async () => {
    const result = await commands.assistantRestoreMissingBuiltins();
    await refreshSettings();
    if (result.status === "error") {
      fail(result.error);
      return;
    }
    toast.success(
      result.data > 0
        ? t("assistantPage.profiles.restored", { count: result.data })
        : t("assistantPage.profiles.restoredNone"),
    );
  };

  const uploadAvatar = async () => {
    try {
      const path = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "Image", extensions: IMAGE_EXTENSIONS }],
      });
      if (typeof path !== "string") return;
      const result = await commands.assistantReadAvatar(path);
      if (result.status === "error") {
        fail(result.error);
        return;
      }
      await patchSelected({ avatar: result.data });
    } catch (error) {
      fail(String(error));
    }
  };

  const importCharacter = async () => {
    try {
      const path = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "Persona", extensions: ["json"] }],
      });
      if (typeof path !== "string") return;
      const result = await commands.assistantImportCharacter(path);
      await refreshSettings();
      if (result.status === "error") {
        fail(result.error);
        return;
      }
      openProfile(result.data.id);
    } catch (error) {
      fail(String(error));
    }
  };

  const exportCharacter = async () => {
    if (!selected) return;
    try {
      const path = await save({
        defaultPath: `${selected.name || "profile"}.json`,
        filters: [{ name: "Persona", extensions: ["json"] }],
      });
      if (!path) return;
      const result = await commands.assistantExportCharacter(selected.id, path);
      if (result.status === "error") fail(result.error);
      else toast.success(t("assistantPage.profiles.exported"));
    } catch (error) {
      fail(String(error));
    }
  };

  const generate = async () => {
    const description = aiDesc.trim();
    if (!description) return;
    setAiLoading(true);
    setAiError(null);
    try {
      const result = await commands.assistantGenerateCharacter(description);
      if (result.status === "error") {
        setAiError(result.error);
        return;
      }
      const id = newId();
      const character: AssistantCharacter = {
        id,
        name: result.data.name,
        prompt: result.data.prompt,
        greeting: result.data.greeting,
        avatar: "",
        kind: "llm",
        builtin: false,
        description: "",
        response_length: null,
      };
      if (await saveCharacters([...characters, character])) {
        setAiDesc("");
        setDictating(false);
        openProfile(id);
      }
    } catch (error) {
      setAiError(String(error));
    } finally {
      setAiLoading(false);
    }
  };

  // First tap starts a hands-free recording, the second stops it; the
  // transcript comes back through `dictation-transcript`.
  const toggleDictation = async () => {
    setDictating((value) => !value);
    try {
      const result = await commands.toggleDictation();
      if (result.status === "error") setDictating(false);
    } catch {
      setDictating(false);
    }
  };

  // Leaving the describe-it pane cancels a recording aimed at it, so nothing
  // keeps listening for a field that is gone.
  const closeAi = () => {
    if (dictating) {
      setDictating(false);
      void commands.cancelOperation().catch(() => {});
    }
    setAiError(null);
    setMode("edit");
  };

  if (!settings) return null;

  const subtitle = (c: AssistantCharacter): string => {
    const description = c.description?.trim();
    if (description) return description;
    if (c.kind === "cat") return t("settings.assistant.characters.meowsOnly");
    return c.builtin
      ? t("settings.assistant.characters.builtin")
      : t("settings.assistant.characters.custom");
  };

  const isCat = selected?.kind === "cat";
  const inUse = selected?.id === activeId;

  const editorMenu: MenuItem[] = selected
    ? [
        {
          id: "duplicate",
          label: t("assistantPage.profiles.duplicate"),
          onSelect: () => void duplicate(),
        },
        {
          id: "export",
          label: t("assistantPage.profiles.export"),
          onSelect: () => void exportCharacter(),
        },
        ...(selected.builtin
          ? [
              {
                id: "restore",
                label: t("assistantPage.profiles.restoreDefault"),
                onSelect: () => void restoreDefault(),
              },
            ]
          : []),
        ...(selected.id !== "default"
          ? [
              {
                id: "delete",
                label: t("assistantPage.profiles.delete"),
                tone: "danger" as const,
                separated: true,
                onSelect: () => setConfirmDelete(true),
              },
            ]
          : []),
      ]
    : [];

  const list = (
    <aside className="flex w-64 shrink-0 flex-col border-e border-hairline">
      <div
        role="listbox"
        aria-label={t("settings.assistant.characters.galleryLabel")}
        className="min-h-0 flex-1 space-y-0.5 overflow-y-auto p-2"
      >
        {characters.map((character) => {
          const isOpen = mode === "edit" && character.id === selected?.id;
          return (
            <button
              key={character.id}
              type="button"
              role="option"
              aria-selected={isOpen}
              onClick={() => openProfile(character.id)}
              className={`flex w-full cursor-pointer items-center gap-2.5 rounded-lg px-2.5 py-2 text-start transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 ${
                isOpen ? "bg-ink/[0.07]" : "hover:bg-ink/[0.04]"
              }`}
            >
              <Avatar character={character} size={30} />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm font-medium text-ink">
                  {character.name}
                </span>
                <span className="block truncate text-xs text-muted">
                  {subtitle(character)}
                </span>
              </span>
              {character.id === activeId && (
                <Check
                  className="h-4 w-4 shrink-0 text-accent"
                  aria-label={t("assistantPage.profiles.inUse")}
                />
              )}
            </button>
          );
        })}
      </div>
      <div className="flex items-center gap-1 border-t border-hairline p-2">
        <MenuButton
          width={236}
          className="inline-flex h-8 min-w-0 flex-1 cursor-pointer items-center justify-center gap-1.5 rounded-lg border border-hairline-strong bg-surface px-3 text-[0.8125rem] font-medium text-ink transition-colors hover:bg-surface-strong focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
          items={[
            {
              id: "blank",
              label: t("assistantPage.profiles.newBlank"),
              onSelect: () => void createBlank(),
            },
            {
              id: "ai",
              label: t("assistantPage.profiles.newWithAi"),
              onSelect: () => setMode("ai"),
            },
          ]}
        >
          <Plus className="h-3.5 w-3.5" aria-hidden="true" />
          {t("assistantPage.profiles.new")}
        </MenuButton>
        <MenuButton
          width={236}
          ariaLabel={t("assistantPage.profiles.listMore")}
          title={t("assistantPage.profiles.listMore")}
          className={ICON_BUTTON}
          items={[
            {
              id: "import",
              label: t("assistantPage.profiles.import"),
              onSelect: () => void importCharacter(),
            },
            {
              id: "restore",
              label: t("assistantPage.profiles.restoreBuiltins"),
              onSelect: () => void restoreMissing(),
            },
          ]}
        >
          <MoreHorizontal className="h-4 w-4" aria-hidden="true" />
        </MenuButton>
      </div>
    </aside>
  );

  const aiPane = (
    <div className="tab-reveal max-w-2xl space-y-4 px-7 py-6">
      <div>
        <h3 className="text-lg font-semibold text-ink">
          {t("assistantPage.profiles.aiTitle")}
        </h3>
        <p className="mt-1 text-sm text-muted">
          {t("assistantPage.profiles.aiBody")}
        </p>
      </div>
      <div className="relative">
        <Textarea
          id={aiId}
          value={aiDesc}
          onChange={(event) => setAiDesc(event.target.value)}
          placeholder={t("settings.assistant.characters.aiPlaceholder")}
          aria-label={t("assistantPage.profiles.aiTitle")}
          rows={5}
          autoFocus
          className="w-full pe-12"
        />
        <button
          type="button"
          // Keep the caret where it is; the transcript arrives as an event.
          onMouseDown={(event) => event.preventDefault()}
          onClick={() => void toggleDictation()}
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
          className={`absolute end-2.5 top-2.5 grid h-8 w-8 cursor-pointer place-items-center rounded-lg border transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 ${
            dictating
              ? "animate-pulse border-transparent bg-accent text-on-primary"
              : "border-hairline-strong bg-surface text-muted hover:text-ink"
          }`}
        >
          {dictating ? (
            <Square className="h-3.5 w-3.5" aria-hidden="true" />
          ) : (
            <Mic className="h-4 w-4" aria-hidden="true" />
          )}
        </button>
      </div>
      <div className="flex items-center gap-2">
        <Button
          size="sm"
          disabled={aiLoading || !aiDesc.trim()}
          onClick={() => void generate()}
        >
          {aiLoading && (
            <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
          )}
          {t("assistantPage.profiles.aiCreate")}
        </Button>
        <Button variant="ghost" size="sm" onClick={closeAi}>
          {t("common.cancel")}
        </Button>
      </div>
      {aiError && (
        <p role="alert" className="text-sm leading-snug text-error">
          {aiError}
        </p>
      )}
    </div>
  );

  const editor = selected && (
    <div key={selected.id} className="tab-reveal max-w-2xl space-y-5 px-7 py-6">
      <header className="flex items-center gap-4">
        <Avatar character={selected} size={56} />
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-lg font-semibold text-ink">
            {selected.name}
          </h3>
          <div className="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-1">
            <button
              type="button"
              onClick={() => void uploadAvatar()}
              className="cursor-pointer rounded text-xs font-medium text-muted transition-colors hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
            >
              {selected.avatar
                ? t("assistantPage.profiles.changePicture")
                : t("assistantPage.profiles.addPicture")}
            </button>
            {selected.avatar && (
              <button
                type="button"
                onClick={() => void patchSelected({ avatar: "" })}
                className="cursor-pointer rounded text-xs font-medium text-muted transition-colors hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
              >
                {t("assistantPage.profiles.removePicture")}
              </button>
            )}
          </div>
        </div>
        {inUse ? (
          <span className="inline-flex shrink-0 items-center gap-1.5 text-[0.8125rem] font-medium text-accent">
            <Check className="h-4 w-4" aria-hidden="true" />
            {t("assistantPage.profiles.inUse")}
          </span>
        ) : (
          <Button size="sm" onClick={() => void activate(selected.id)}>
            {t("assistantPage.profiles.use")}
          </Button>
        )}
        <MenuButton
          width={236}
          ariaLabel={t("assistantPage.profiles.more")}
          title={t("assistantPage.profiles.more")}
          className={ICON_BUTTON}
          items={editorMenu}
        >
          <MoreHorizontal className="h-4 w-4" aria-hidden="true" />
        </MenuButton>
      </header>

      <Field
        label={t("settings.assistant.characters.nameLabel")}
        htmlFor={nameId}
      >
        <Input
          ref={nameRef}
          id={nameId}
          value={draftName}
          onChange={(event) => setDraftName(event.target.value)}
          onBlur={() => {
            const name = draftName.trim();
            if (name && name !== selected.name) void patchSelected({ name });
            else setDraftName(selected.name);
          }}
          className="w-full"
        />
      </Field>

      <Field
        label={t("assistantPage.profiles.roleLabel")}
        tip={t("assistantPage.profiles.roleTip")}
        htmlFor={roleId}
      >
        <Input
          id={roleId}
          value={draftDescription}
          onChange={(event) => setDraftDescription(event.target.value)}
          onBlur={() => {
            const description = draftDescription.trim();
            if (description !== (selected.description ?? "")) {
              void patchSelected({ description });
            }
          }}
          placeholder={t("assistantPage.profiles.rolePlaceholder")}
          className="w-full"
        />
      </Field>

      {isCat ? (
        <p className="rounded-xl bg-surface-muted px-4 py-3 text-sm leading-relaxed text-muted">
          {t("settings.assistant.characters.catNote")}
        </p>
      ) : (
        <>
          <Field
            label={t("assistantPage.profiles.instructionsLabel")}
            tip={t("assistantPage.profiles.instructionsTip")}
            htmlFor={promptId}
          >
            <Textarea
              id={promptId}
              value={draftPrompt}
              onChange={(event) => setDraftPrompt(event.target.value)}
              onBlur={() => {
                if (draftPrompt !== selected.prompt) {
                  void patchSelected({ prompt: draftPrompt });
                }
              }}
              rows={9}
              className="w-full"
            />
          </Field>
          <Field
            label={t("assistantPage.profiles.lengthLabel")}
            tip={t("assistantPage.profiles.lengthTip")}
          >
            <div>
              <Segmented
                label={t("assistantPage.profiles.lengthLabel")}
                value={(selected.response_length ?? "inherit") as LengthChoice}
                onChange={(value) =>
                  void patchSelected({
                    response_length: value === "inherit" ? null : value,
                  })
                }
                options={LENGTHS.map((value) => ({
                  value,
                  label: t(`assistantPage.profiles.lengths.${value}`),
                }))}
              />
            </div>
          </Field>
        </>
      )}

      <Field
        label={t("assistantPage.profiles.greetingLabel")}
        tip={t("assistantPage.profiles.greetingTip")}
        htmlFor={greetingId}
      >
        <Input
          id={greetingId}
          value={draftGreeting}
          onChange={(event) => setDraftGreeting(event.target.value)}
          onBlur={() => {
            if (draftGreeting !== selected.greeting) {
              void patchSelected({ greeting: draftGreeting });
            }
          }}
          placeholder={t("assistantPage.profiles.greetingPlaceholder")}
          className="w-full"
        />
      </Field>
    </div>
  );

  return (
    <div className="flex min-h-0 min-w-0 flex-1">
      {list}
      <div className="min-w-0 flex-1 overflow-y-auto">
        {mode === "ai" ? aiPane : editor}
      </div>
      <Dialog
        open={confirmDelete && !!selected}
        onClose={() => setConfirmDelete(false)}
        size="sm"
        title={t("assistantPage.profiles.deleteTitle", {
          name: selected?.name ?? "",
        })}
        footer={
          <>
            <Button
              variant="ghost"
              size="sm"
              className="ms-auto"
              onClick={() => setConfirmDelete(false)}
            >
              {t("common.cancel")}
            </Button>
            <Button variant="danger" size="sm" onClick={() => void remove()}>
              {t("common.delete")}
            </Button>
          </>
        }
      >
        <p className="text-sm leading-relaxed text-body">
          {t("assistantPage.profiles.deleteBody")}
        </p>
      </Dialog>
    </div>
  );
};

export default CharactersSettings;
