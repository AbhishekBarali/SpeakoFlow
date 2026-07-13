import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  ChevronDown,
  Download,
  Loader2,
  Pencil,
  Plus,
  Sparkles,
  Trash2,
  Upload,
} from "lucide-react";
import { commands, type MemoryDetail, type MemoryNote } from "@/bindings";
import { Button } from "@/components/ui/Button";
import { Input } from "../../ui/Input";
import { Textarea } from "@/components/ui";
import { Dropdown } from "../../ui/Dropdown";
import { ToggleSwitch } from "../../ui/ToggleSwitch";
import { useSettings } from "../../../hooks/useSettings";

/** A card whose body is hidden until you click the header — keeps the page
 *  short and makes edits deliberate (content is read-first, not sitting open).
 *  Title + a one-line caption sit on the left; an optional meta hint (preview /
 *  count) and the chevron sit on the right. */
const CollapsibleSection: React.FC<{
  title: string;
  caption: string;
  meta?: string;
  open: boolean;
  onToggle: () => void;
  children: React.ReactNode;
}> = ({ title, caption, meta, open, onToggle, children }) => (
  <div className="rounded-xl border border-hairline bg-surface">
    <button
      type="button"
      onClick={onToggle}
      aria-expanded={open}
      className="flex w-full items-center gap-3 px-4 py-3 text-start cursor-pointer"
    >
      <span className="min-w-0 flex-1">
        <span className="block text-[13px] font-medium text-ink">{title}</span>
        <span className="mt-0.5 block text-xs text-muted leading-snug">
          {caption}
        </span>
      </span>
      {meta && (
        <span className="ms-auto max-w-[38%] shrink-0 truncate text-xs text-muted">
          {meta}
        </span>
      )}
      <ChevronDown
        size={16}
        className={`shrink-0 text-muted transition-transform duration-200 ${
          open ? "rotate-180" : ""
        }`}
      />
    </button>
    {open && (
      <div className="border-t border-hairline px-4 py-4">{children}</div>
    )}
  </div>
);

/**
 * Personal memory ("About You") — the Memory sub-page. A local-first,
 * user-owned profile the assistant can draw on: an always-on summary plus a
 * list of durable notes. Everything here is inspectable, editable, exportable,
 * and off by default. Distillation (learning from a chat) runs on the backend;
 * this page is the transparency + control surface.
 */
export const MemorySettings: React.FC = () => {
  const { t } = useTranslation();
  const { settings, refreshSettings } = useSettings();

  const enabled = settings?.assistant_memory_enabled ?? false;
  const incognito = settings?.assistant_memory_incognito ?? false;
  const detail = (settings?.assistant_memory_detail ??
    "balanced") as MemoryDetail;
  const memory = settings?.assistant_memory;
  const notes = memory?.notes ?? [];

  const [aboutDraft, setAboutDraft] = useState("");
  const [newNote, setNewNote] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [distilling, setDistilling] = useState(false);
  const [confirmWipe, setConfirmWipe] = useState(false);
  // Summary + Notes are collapsed by default (read-first, no accidental edits).
  const [summaryOpen, setSummaryOpen] = useState(false);
  const [notesOpen, setNotesOpen] = useState(false);
  const [editingSummary, setEditingSummary] = useState(false);

  // Keep the About You draft in sync with the stored value when it changes
  // (e.g. after a distillation pass refreshes it).
  useEffect(() => {
    setAboutDraft(memory?.about_you ?? "");
  }, [memory?.about_you]);

  /** Run a command that returns a Result, surface errors, then refresh. */
  const run = useCallback(
    async (
      action: () => Promise<{ status: "ok" | "error"; error?: string }>,
    ): Promise<boolean> => {
      const res = await action();
      if (res.status === "error") {
        setError(res.error ?? "Something went wrong.");
        return false;
      }
      setError(null);
      await refreshSettings();
      return true;
    },
    [refreshSettings],
  );

  const toggleEnabled = useCallback(
    (value: boolean) => {
      void run(() => commands.setAssistantMemoryEnabled(value));
    },
    [run],
  );

  const toggleIncognito = useCallback(
    (value: boolean) => {
      void run(() => commands.setAssistantMemoryIncognito(value));
    },
    [run],
  );

  const changeDetail = useCallback(
    (value: string) => {
      void run(() => commands.setAssistantMemoryDetail(value as MemoryDetail));
    },
    [run],
  );

  const saveAbout = useCallback(() => {
    const next = aboutDraft.trim();
    if (next === (memory?.about_you ?? "").trim()) return;
    void run(() => commands.setAssistantMemoryAboutYou(next));
  }, [aboutDraft, memory?.about_you, run]);

  const addNote = useCallback(async () => {
    const text = newNote.trim();
    if (!text) return;
    const ok = await run(() => commands.addAssistantMemoryNote(text));
    if (ok) setNewNote("");
  }, [newNote, run]);

  const saveNote = useCallback(
    (note: MemoryNote, text: string) => {
      const next = text.trim();
      if (!next || next === note.text.trim()) return;
      void run(() => commands.updateAssistantMemoryNote(note.id, next));
    },
    [run],
  );

  const deleteNote = useCallback(
    (id: string) => {
      void run(() => commands.deleteAssistantMemoryNote(id));
    },
    [run],
  );

  const wipe = useCallback(async () => {
    const ok = await run(() => commands.clearAssistantMemory());
    if (ok) setConfirmWipe(false);
  }, [run]);

  const distillNow = useCallback(async () => {
    setDistilling(true);
    setError(null);
    try {
      const res = await commands.assistantDistillMemoryNow();
      if (res.status === "error") {
        setError(res.error);
        return;
      }
      await refreshSettings();
    } catch (err) {
      setError(String(err));
    } finally {
      setDistilling(false);
    }
  }, [refreshSettings]);

  const exportMemory = useCallback(async () => {
    try {
      const path = await save({
        defaultPath: "speakoflow-memory.json",
        filters: [{ name: "Memory", extensions: ["json"] }],
      });
      if (!path) return;
      await run(() => commands.exportAssistantMemory(path));
    } catch (err) {
      setError(String(err));
    }
  }, [run]);

  const importMemory = useCallback(async () => {
    try {
      const path = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "Memory", extensions: ["json"] }],
      });
      if (typeof path !== "string") return;
      await run(() => commands.importAssistantMemory(path));
    } catch (err) {
      setError(String(err));
    }
  }, [run]);

  if (!settings) return null;

  const summaryText = (memory?.about_you ?? "").trim();
  const summaryPreview = summaryText
    ? summaryText.length > 46
      ? `${summaryText.slice(0, 46)}…`
      : summaryText
    : t("settings.personalMemory.aboutYou.collapsedEmpty");

  const confidenceLabel = (note: MemoryNote): string => {
    switch (note.confidence) {
      case "high":
        return t("settings.personalMemory.confidence.high");
      case "low":
        return t("settings.personalMemory.confidence.low");
      default:
        return t("settings.personalMemory.confidence.medium");
    }
  };

  return (
    <div className="max-w-3xl w-full mx-auto space-y-8">
      {/* Hero toggles ---------------------------------------------------- */}
      <section>
        <div className="rounded-xl border border-hairline bg-surface divide-y divide-hairline">
          <ToggleSwitch
            checked={enabled}
            onChange={toggleEnabled}
            label={t("settings.personalMemory.enable.label")}
            description={t("settings.personalMemory.enable.description")}
            grouped
          />
          <ToggleSwitch
            checked={incognito}
            onChange={toggleIncognito}
            label={t("settings.personalMemory.incognito.label")}
            description={t("settings.personalMemory.incognito.description")}
            grouped
          />
        </div>
      </section>

      {/* About You (collapsible, read-first) ----------------------------- */}
      <CollapsibleSection
        title={t("settings.personalMemory.aboutYou.label")}
        caption={t("settings.personalMemory.aboutYou.caption")}
        meta={summaryOpen ? undefined : summaryPreview}
        open={summaryOpen}
        onToggle={() => setSummaryOpen((v) => !v)}
      >
        {editingSummary ? (
          <div className="space-y-3">
            <Textarea
              value={aboutDraft}
              onChange={(e) => setAboutDraft(e.target.value)}
              onBlur={saveAbout}
              rows={4}
              autoFocus
              placeholder={t("settings.personalMemory.aboutYou.placeholder")}
              className="w-full"
            />
            <div className="flex items-start justify-between gap-3">
              <p className="text-[11px] text-muted leading-relaxed max-w-md">
                {t("settings.personalMemory.aboutYou.hint")}
              </p>
              <Button
                variant="secondary"
                size="sm"
                onClick={() => {
                  saveAbout();
                  setEditingSummary(false);
                }}
              >
                {t("settings.personalMemory.aboutYou.done")}
              </Button>
            </div>
          </div>
        ) : (
          <div className="space-y-3">
            <p
              className={`text-[13px] leading-relaxed whitespace-pre-wrap ${
                summaryText ? "text-body" : "text-muted"
              }`}
            >
              {summaryText || t("settings.personalMemory.aboutYou.empty")}
            </p>
            <Button
              variant="secondary"
              size="sm"
              onClick={() => setEditingSummary(true)}
            >
              <Pencil size={14} />
              {t("settings.personalMemory.aboutYou.edit")}
            </Button>
          </div>
        )}
      </CollapsibleSection>

      {/* Notes (collapsible, scrollable) --------------------------------- */}
      <CollapsibleSection
        title={t("settings.personalMemory.notes.label")}
        caption={t("settings.personalMemory.notes.caption")}
        meta={notes.length > 0 ? String(notes.length) : undefined}
        open={notesOpen}
        onToggle={() => setNotesOpen((v) => !v)}
      >
        <div className="space-y-3">
          {notes.length === 0 ? (
            <p className="text-xs text-muted leading-relaxed">
              {t("settings.personalMemory.notes.empty")}
            </p>
          ) : (
            <ul className="space-y-2 max-h-64 overflow-y-auto pr-1">
              {notes.map((note) => (
                <li key={note.id} className="flex items-start gap-2">
                  <div className="flex-1 min-w-0">
                    <Input
                      type="text"
                      defaultValue={note.text}
                      onBlur={(e) => saveNote(note, e.target.value)}
                      className="w-full"
                    />
                    <span className="mt-1 block text-[10.5px] uppercase tracking-wide text-muted-soft">
                      {confidenceLabel(note)}
                      {note.source === "auto"
                        ? ` · ${t("settings.personalMemory.notes.learned")}`
                        : ` · ${t("settings.personalMemory.notes.added")}`}
                    </span>
                  </div>
                  <button
                    type="button"
                    onClick={() => deleteNote(note.id)}
                    title={t("settings.personalMemory.notes.delete")}
                    aria-label={t("settings.personalMemory.notes.delete")}
                    className="mt-1.5 shrink-0 text-muted hover:text-error transition-colors cursor-pointer"
                  >
                    <Trash2 size={15} />
                  </button>
                </li>
              ))}
            </ul>
          )}

          <div className="flex items-center gap-2 pt-1">
            <Input
              type="text"
              value={newNote}
              onChange={(e) => setNewNote(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  void addNote();
                }
              }}
              placeholder={t("settings.personalMemory.notes.addPlaceholder")}
              className="flex-1"
            />
            <Button
              variant="secondary"
              size="sm"
              onClick={addNote}
              disabled={!newNote.trim()}
            >
              <Plus size={14} />
              {t("settings.personalMemory.notes.add")}
            </Button>
          </div>
        </div>
      </CollapsibleSection>

      {/* Detail + update now + export + import ---------------------------- */}
      <section className="space-y-3">
        <div className="rounded-xl border border-hairline bg-surface divide-y divide-hairline">
          <div className="flex items-center gap-3 px-4 py-3">
            <div className="min-w-0 flex-1">
              <h3 className="text-[13px] font-normal leading-snug text-ink">
                {t("settings.personalMemory.detail.label")}
              </h3>
              <p className="mt-0.5 text-xs leading-snug text-muted max-w-md">
                {t("settings.personalMemory.detail.description")}
              </p>
            </div>
            <div className="relative shrink-0">
              <Dropdown
                options={[
                  {
                    value: "light",
                    label: t("settings.personalMemory.detail.options.light"),
                  },
                  {
                    value: "balanced",
                    label: t("settings.personalMemory.detail.options.balanced"),
                  },
                  {
                    value: "detailed",
                    label: t("settings.personalMemory.detail.options.detailed"),
                  },
                ]}
                selectedValue={detail}
                onSelect={changeDetail}
              />
            </div>
          </div>

          <div className="flex items-center justify-between gap-3 px-4 py-3">
            <div className="min-w-0 flex-1">
              <h3 className="text-[13px] font-normal leading-snug text-ink">
                {t("settings.personalMemory.distill.label")}
              </h3>
              <p className="mt-0.5 text-xs leading-snug text-muted max-w-md">
                {t("settings.personalMemory.distill.description")}
              </p>
            </div>
            <Button
              variant="secondary"
              size="sm"
              onClick={distillNow}
              disabled={distilling || !enabled}
            >
              {distilling ? (
                <Loader2 size={14} className="animate-spin" />
              ) : (
                <Sparkles size={14} />
              )}
              {t("settings.personalMemory.distill.button")}
            </Button>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-2 px-0.5">
          <Button variant="secondary" size="sm" onClick={exportMemory}>
            <Download size={14} />
            {t("settings.personalMemory.export")}
          </Button>
          <Button variant="secondary" size="sm" onClick={importMemory}>
            <Upload size={14} />
            {t("settings.personalMemory.import")}
          </Button>
        </div>
      </section>

      {/* Destructive wipe row -------------------------------------------- */}
      <section className="space-y-2">
        <div className="rounded-xl border border-hairline bg-surface px-4 py-3">
          {confirmWipe ? (
            <div className="flex items-center justify-between gap-3">
              <span className="text-xs text-muted">
                {t("settings.personalMemory.wipe.confirm")}
              </span>
              <div className="flex items-center gap-2">
                <Button variant="danger-ghost" size="sm" onClick={wipe}>
                  {t("settings.personalMemory.wipe.yes")}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => setConfirmWipe(false)}
                >
                  {t("settings.personalMemory.wipe.cancel")}
                </Button>
              </div>
            </div>
          ) : (
            <button
              type="button"
              onClick={() => setConfirmWipe(true)}
              className="flex w-full items-center gap-2 text-[13px] font-medium text-error hover:opacity-80 transition-opacity cursor-pointer"
            >
              <Trash2 size={14} />
              {t("settings.personalMemory.wipe.button")}
            </button>
          )}
        </div>
        {error && <p className="text-xs text-error px-0.5">{error}</p>}
      </section>
    </div>
  );
};

export default MemorySettings;
