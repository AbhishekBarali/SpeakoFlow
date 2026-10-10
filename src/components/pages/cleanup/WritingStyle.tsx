import React, { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Check, Pencil, Plus, Trash2 } from "lucide-react";
import { commands, type CustomPostProcessTone } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Textarea } from "@/components/ui/Textarea";
import { InfoTip } from "@/components/ui/InfoTip";
import { Callout } from "@/components/ui/Callout";
import { useNavigation } from "@/components/shell/navigation";
import { useCleanupSpecialist } from "./useCleanupSpecialist";

/** Mirrors `PostProcessTone` ids in settings.rs. */
const BUILTIN_TONE_IDS = [
  "none",
  "professional",
  "friendly",
  "concise",
  "formal",
  "casual",
] as const;
type BuiltinTone = (typeof BUILTIN_TONE_IDS)[number];
const BUILTIN_IDS = new Set<string>(BUILTIN_TONE_IDS);

type EditorState =
  | { mode: "create" }
  | { mode: "edit"; tone: CustomPostProcessTone }
  | null;

/** The user's own styles, minus anything malformed. */
const useCustomTones = (): CustomPostProcessTone[] => {
  const { getSetting } = useSettings();
  return (getSetting("post_process_custom_tones") ?? []).filter(
    (tone) =>
      tone.id.trim().length > 0 &&
      !BUILTIN_IDS.has(tone.id) &&
      tone.name.trim() &&
      tone.instruction.trim(),
  );
};

/** The frame every state of the area under the pills shares. */
const PANEL = "mt-4 rounded-xl border border-hairline";

/**
 * A style's name and instructions, stacked, in the same place the preview
 * sits — so creating or editing a style swaps what is under the pills rather
 * than growing a second box below them.
 */
const StyleEditor: React.FC<{
  state: Exclude<EditorState, null>;
  onDone: () => void;
}> = ({ state, onDone }) => {
  const { t } = useTranslation();
  const { refreshSettings } = useSettings();
  const isEdit = state.mode === "edit";
  const [name, setName] = useState(isEdit ? state.tone.name : "");
  const [instruction, setInstruction] = useState(
    isEdit ? state.tone.instruction : "",
  );
  const [busy, setBusy] = useState(false);
  const nameId = useId();
  const instructionId = useId();
  const nameRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    nameRef.current?.focus();
  }, []);

  const fail = (key: string) => toast.error(t(key));

  const save = async () => {
    if (!name.trim() || !instruction.trim() || busy) return;
    setBusy(true);
    try {
      if (state.mode === "create") {
        const created = await commands.addPostProcessCustomTone(
          name.trim(),
          instruction.trim(),
        );
        if (created.status !== "ok") {
          fail("settings.postProcessing.errors.toneCreateFailed");
          return;
        }
        const selected = await commands.changePostProcessToneSetting(
          created.data.id,
        );
        if (selected.status !== "ok") {
          fail("settings.postProcessing.errors.toneSelectAfterCreateFailed");
        }
      } else {
        const updated = await commands.updatePostProcessCustomTone(
          state.tone.id,
          name.trim(),
          instruction.trim(),
        );
        if (updated.status !== "ok") {
          fail("settings.postProcessing.errors.toneUpdateFailed");
          return;
        }
      }
      await refreshSettings();
      onDone();
    } catch (error) {
      console.error("Failed to save writing style:", error);
      fail(
        state.mode === "create"
          ? "settings.postProcessing.errors.toneCreateFailed"
          : "settings.postProcessing.errors.toneUpdateFailed",
      );
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    if (state.mode !== "edit" || busy) return;
    setBusy(true);
    try {
      const result = await commands.deletePostProcessCustomTone(state.tone.id);
      if (result.status !== "ok") {
        fail("settings.postProcessing.errors.toneDeleteFailed");
        return;
      }
      // Deleting the selected style falls back to "None" in the backend.
      await refreshSettings();
      onDone();
    } catch (error) {
      console.error("Failed to delete writing style:", error);
      fail("settings.postProcessing.errors.toneDeleteFailed");
    } finally {
      setBusy(false);
    }
  };

  const unchanged =
    isEdit &&
    name.trim() === state.tone.name &&
    instruction.trim() === state.tone.instruction;

  return (
    <form
      className={`${PANEL} tab-reveal space-y-4 p-4 sm:p-5`}
      onSubmit={(event) => {
        event.preventDefault();
        void save();
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          onDone();
        }
      }}
    >
      <p className="text-sm font-semibold text-ink">
        {isEdit
          ? t("cleanup.styles.editTitle")
          : t("cleanup.styles.createTitle")}
      </p>
      <div className="space-y-1.5">
        <label
          htmlFor={nameId}
          className="block text-[0.8125rem] font-medium text-ink"
        >
          {t("cleanup.styles.form.name")}
        </label>
        <Input
          ref={nameRef}
          id={nameId}
          value={name}
          onChange={(event) => setName(event.target.value)}
          placeholder={t("settings.postProcessing.tone.namePlaceholder")}
          className="w-full"
        />
      </div>
      <div className="space-y-1.5">
        <div className="flex items-center gap-1">
          <label
            htmlFor={instructionId}
            className="text-[0.8125rem] font-medium text-ink"
          >
            {t("cleanup.styles.form.instruction")}
          </label>
          <InfoTip text={t("cleanup.styles.form.instructionTip")} />
        </div>
        <Textarea
          id={instructionId}
          value={instruction}
          onChange={(event) => setInstruction(event.target.value)}
          rows={4}
          placeholder={t("cleanup.styles.form.instructionPlaceholder")}
          className="w-full"
        />
      </div>
      <div className="flex items-center gap-2 pt-1">
        {isEdit && (
          <Button
            type="button"
            variant="danger-ghost"
            size="sm"
            onClick={() => void remove()}
            disabled={busy}
          >
            <Trash2 className="h-3.5 w-3.5" aria-hidden="true" />
            {t("settings.postProcessing.tone.deleteTone")}
          </Button>
        )}
        <div className="ms-auto flex items-center gap-2">
          <Button type="button" variant="ghost" size="sm" onClick={onDone}>
            {t("common.cancel")}
          </Button>
          <Button
            type="submit"
            size="sm"
            disabled={!name.trim() || !instruction.trim() || busy || unchanged}
          >
            {isEdit
              ? t("settings.postProcessing.tone.updateTone")
              : t("settings.postProcessing.tone.createTone")}
          </Button>
        </div>
      </div>
    </form>
  );
};

/** A built-in style, shown on one real sentence: what you say, what it types. */
const BuiltinPreview: React.FC<{ tone: BuiltinTone }> = ({ tone }) => {
  const { t } = useTranslation();
  return (
    <>
      <div
        className={`${PANEL} grid gap-px overflow-hidden bg-hairline sm:grid-cols-2`}
      >
        <div className="bg-surface-muted px-4 py-3.5">
          <p className="text-xs font-medium text-muted">
            {t("cleanup.styles.preview.youSay")}
          </p>
          <p className="mt-1.5 text-sm leading-relaxed text-muted">
            {t("cleanup.styles.samples.raw")}
          </p>
        </div>
        <div className="bg-surface px-4 py-3.5">
          <p className="text-xs font-medium text-accent">
            {t("cleanup.styles.preview.itTypes")}
          </p>
          <p
            key={tone}
            className="style-swap mt-1.5 text-[0.9375rem] font-medium leading-relaxed text-ink"
          >
            {t(`cleanup.styles.samples.${tone}`)}
          </p>
        </div>
      </div>
      <p className="mt-2.5 text-xs text-muted">
        {t(`cleanup.styles.builtin.${tone}`)}
      </p>
    </>
  );
};

/**
 * A style of your own: its instruction, which is the whole of what it does.
 * It cannot be previewed honestly on a sample sentence without running it, so
 * there is no "you say" beside it.
 */
const CustomStyleView: React.FC<{
  tone: CustomPostProcessTone;
  onEdit: () => void;
  disabled: boolean;
}> = ({ tone, onEdit, disabled }) => {
  const { t } = useTranslation();
  return (
    <div key={tone.id} className={`${PANEL} style-swap px-4 py-3.5`}>
      <div className="flex min-h-7 items-center gap-2">
        <p className="min-w-0 flex-1 text-xs font-medium text-accent">
          {t("cleanup.styles.preview.yourInstruction")}
        </p>
        <Button
          variant="ghost"
          size="sm"
          onClick={onEdit}
          disabled={disabled}
          className="-me-2"
        >
          <Pencil className="h-3.5 w-3.5" aria-hidden="true" />
          {t("common.edit")}
        </Button>
      </div>
      <p className="mt-1 whitespace-pre-wrap text-sm leading-relaxed text-ink">
        {tone.instruction}
      </p>
    </div>
  );
};

/**
 * Said under the pills while a style is chosen but cleanup runs on a fine-tune.
 *
 * A cleanup fine-tune was trained on one exact prompt with no style, and in
 * practice returns plain cleanup whatever style is sent with it. The style is
 * still sent (it is the user's choice), so without this the pills look broken.
 */
const SpecialistIgnoresStyle: React.FC<{ name: string }> = ({ name }) => {
  const { t } = useTranslation();
  const { openModelSlot } = useNavigation();
  return (
    <Callout
      tone="info"
      className="mt-4"
      action={
        <Button
          variant="secondary"
          size="sm"
          onClick={() => openModelSlot("cleanup")}
        >
          {t("cleanup.styles.specialist.changeModel")}
        </Button>
      }
    >
      {t("cleanup.styles.specialist.ignored", { model: name })}
    </Callout>
  );
};

/**
 * Writing style, with every style in plain sight.
 *
 * Each style is a pill. Under the pills: a built-in style shows what it does
 * to one real sentence, a style of your own shows its instruction, and
 * creating or editing one turns that same area into the form.
 */
export const WritingStyleCard: React.FC<{ disabled?: boolean }> = ({
  disabled = false,
}) => {
  const { t } = useTranslation();
  const { getSetting, refreshSettings } = useSettings();
  const customTones = useCustomTones();
  const [editor, setEditor] = useState<EditorState>(null);
  const [busy, setBusy] = useState(false);
  const selectedId =
    getSetting("post_process_selected_tone_id") ??
    getSetting("post_process_tone") ??
    "none";
  const selectedCustom = customTones.find((tone) => tone.id === selectedId);
  const builtin: BuiltinTone = BUILTIN_IDS.has(selectedId)
    ? (selectedId as BuiltinTone)
    : "none";
  // What the backend will actually send: an unknown or deleted id resolves to
  // no style there, so it is no style here too.
  const styleChosen = !!selectedCustom || builtin !== "none";
  const specialist = useCleanupSpecialist();

  const select = async (toneId: string) => {
    setEditor(null);
    if (toneId === selectedId || busy) return;
    setBusy(true);
    try {
      const result = await commands.changePostProcessToneSetting(toneId);
      if (result.status !== "ok") {
        toast.error(t("settings.postProcessing.errors.toneSelectFailed"));
      }
    } catch (error) {
      console.error("Failed to select writing style:", error);
      toast.error(t("settings.postProcessing.errors.toneSelectFailed"));
    } finally {
      await refreshSettings();
      setBusy(false);
    }
  };

  const pill = (id: string, label: string) => {
    const selected = id === selectedId && !editor;
    return (
      <button
        key={id}
        type="button"
        role="radio"
        aria-checked={id === selectedId}
        disabled={disabled}
        onClick={() => void select(id)}
        className={`inline-flex h-9 cursor-pointer items-center gap-1.5 rounded-full border px-3.5 text-[0.8125rem] font-medium transition-[background-color,border-color,color] duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 disabled:cursor-not-allowed disabled:opacity-50 ${
          selected
            ? "border-accent bg-accent text-on-primary"
            : "border-hairline-strong bg-surface text-body hover:border-ink/25 hover:text-ink"
        }`}
      >
        {selected && (
          <Check className="h-3.5 w-3.5" strokeWidth={2.5} aria-hidden="true" />
        )}
        {label}
      </button>
    );
  };

  const creating = editor?.mode === "create";

  return (
    <div>
      <div
        role="radiogroup"
        aria-label={t("cleanup.styles.title")}
        className="flex flex-wrap gap-2"
      >
        {BUILTIN_TONE_IDS.map((id) =>
          pill(id, t(`settings.postProcessing.tone.options.${id}`)),
        )}
        {customTones.map((tone) => pill(tone.id, tone.name))}
        <button
          type="button"
          aria-pressed={creating}
          onClick={() => setEditor(creating ? null : { mode: "create" })}
          disabled={disabled}
          className={`inline-flex h-9 cursor-pointer items-center gap-1.5 rounded-full border border-dashed px-3.5 text-[0.8125rem] font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 disabled:cursor-not-allowed disabled:opacity-50 ${
            creating
              ? "border-accent text-accent"
              : "border-hairline-strong text-muted hover:border-accent/50 hover:text-accent"
          }`}
        >
          <Plus className="h-3.5 w-3.5" aria-hidden="true" />
          {t("cleanup.styles.newStyle")}
        </button>
      </div>

      {styleChosen && specialist && (
        <SpecialistIgnoresStyle name={specialist.name} />
      )}

      {editor ? (
        <StyleEditor
          key={editor.mode === "edit" ? editor.tone.id : "create"}
          state={editor}
          onDone={() => setEditor(null)}
        />
      ) : selectedCustom ? (
        <CustomStyleView
          tone={selectedCustom}
          disabled={disabled}
          onEdit={() => setEditor({ mode: "edit", tone: selectedCustom })}
        />
      ) : (
        <BuiltinPreview tone={builtin} />
      )}
    </div>
  );
};
