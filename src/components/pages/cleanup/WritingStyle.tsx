import React, { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Check, Pencil, Plus, Trash2 } from "lucide-react";
import { commands, type CustomPostProcessTone } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Textarea } from "@/components/ui/Textarea";

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

/** Name and instruction for one style, edited in place inside the card. */
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
      className="mt-4 space-y-3 rounded-xl border border-accent/30 bg-accent/[0.03] p-4"
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
      <div className="grid gap-3 sm:grid-cols-[12rem_minmax(0,1fr)]">
        <div className="space-y-1.5">
          <label
            htmlFor={nameId}
            className="block text-xs font-medium text-muted"
          >
            {t("settings.postProcessing.tone.nameLabel")}
          </label>
          <Input
            ref={nameRef}
            id={nameId}
            value={name}
            onChange={(event) => setName(event.target.value)}
            placeholder={t("settings.postProcessing.tone.namePlaceholder")}
            variant="compact"
            className="w-full"
          />
        </div>
        <div className="space-y-1.5">
          <label
            htmlFor={instructionId}
            className="block text-xs font-medium text-muted"
          >
            {t("settings.postProcessing.tone.instructionsLabel")}
          </label>
          <Textarea
            id={instructionId}
            value={instruction}
            onChange={(event) => setInstruction(event.target.value)}
            rows={3}
            variant="compact"
            placeholder={t(
              "settings.postProcessing.tone.instructionsPlaceholder",
            )}
            className="w-full"
          />
        </div>
      </div>
      <div className="flex items-center gap-2">
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

/**
 * Writing style, with every style in plain sight.
 *
 * Styles used to live in a dropdown with a "Manage" button that opened a
 * dialog, which opened another dialog to edit one — so changing how your
 * dictation reads took three clicks into places nobody looks. Now each style is
 * a pill, the one you pick shows what it does to a real sentence right under
 * it, and your own styles are created and edited in the same card.
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

  const select = async (toneId: string) => {
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
    const selected = id === selectedId;
    return (
      <button
        key={id}
        type="button"
        role="radio"
        aria-checked={selected}
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

  const builtin = BUILTIN_IDS.has(selectedId)
    ? (selectedId as BuiltinTone)
    : null;

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
          onClick={() => setEditor({ mode: "create" })}
          disabled={disabled}
          className="inline-flex h-9 cursor-pointer items-center gap-1.5 rounded-full border border-dashed border-hairline-strong px-3.5 text-[0.8125rem] font-medium text-muted transition-colors hover:border-accent/50 hover:text-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 disabled:cursor-not-allowed disabled:opacity-50"
        >
          <Plus className="h-3.5 w-3.5" aria-hidden="true" />
          {t("cleanup.styles.newStyle")}
        </button>
      </div>

      {/* What the selected style does, shown on one real sentence. A style of
          your own cannot be previewed honestly, so it shows its instruction. */}
      <div className="mt-4 grid gap-px overflow-hidden rounded-xl border border-hairline bg-hairline sm:grid-cols-2">
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
            {selectedCustom
              ? t("cleanup.styles.preview.yourInstruction")
              : t("cleanup.styles.preview.itTypes")}
          </p>
          <p
            key={selectedId}
            className={`style-swap mt-1.5 leading-relaxed text-ink ${
              selectedCustom ? "text-sm" : "text-[0.9375rem] font-medium"
            }`}
          >
            {selectedCustom
              ? selectedCustom.instruction
              : t(`cleanup.styles.samples.${builtin ?? "none"}`)}
          </p>
        </div>
      </div>
      <div className="mt-2.5 flex min-h-8 flex-wrap items-center justify-between gap-2">
        <p className="text-xs text-muted">
          {builtin
            ? t(`cleanup.styles.builtin.${builtin}`)
            : selectedCustom
              ? t("cleanup.styles.customHint")
              : null}
        </p>
        {selectedCustom && !editor && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setEditor({ mode: "edit", tone: selectedCustom })}
          >
            <Pencil className="h-3.5 w-3.5" aria-hidden="true" />
            {t("cleanup.styles.editNamed", { name: selectedCustom.name })}
          </Button>
        )}
      </div>

      {editor && (
        <StyleEditor
          key={editor.mode === "edit" ? editor.tone.id : "create"}
          state={editor}
          onDone={() => setEditor(null)}
        />
      )}
    </div>
  );
};
