import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  Check,
  Download,
  Loader2,
  Pencil,
  Plus,
  Search,
  Sparkles,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import { commands, type MemoryDetail, type MemoryNote } from "@/bindings";
import { Button } from "@/components/ui/Button";
import { Input } from "../../ui/Input";
import { Textarea } from "@/components/ui";
import { Segmented } from "@/components/ui/Segmented";
import { Switch } from "@/components/ui/Switch";
import { InfoTip } from "@/components/ui/InfoTip";
import { SettingsGroup } from "@/components/ui/SettingsGroup";
import { SettingContainer } from "@/components/ui/SettingContainer";
import { useSettings } from "../../../hooks/useSettings";

type NoteFilter = "all" | "auto" | "user";

/** A note's text as a plain line; click the pencil (or the line) to edit it in place. */
const NoteRow: React.FC<{
  note: MemoryNote;
  onSave: (note: MemoryNote, text: string) => void;
  onDelete: (id: string) => void;
}> = ({ note, onSave, onDelete }) => {
  const { t } = useTranslation();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(note.text);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!editing) setDraft(note.text);
  }, [note.text, editing]);
  useEffect(() => {
    if (editing) inputRef.current?.select();
  }, [editing]);

  const commit = () => {
    setEditing(false);
    if (draft.trim() && draft.trim() !== note.text.trim()) onSave(note, draft);
    else setDraft(note.text);
  };

  const learned = note.source === "auto";
  const confidence = note.confidence ?? "medium";

  return (
    <li className="group flex items-center gap-3 px-4 py-2.5 transition-colors hover:bg-surface-muted">
      <span
        aria-hidden="true"
        title={t(`settings.personalMemory.confidence.${confidence}`)}
        className={`h-1.5 w-1.5 shrink-0 rounded-full ${
          confidence === "high"
            ? "bg-accent"
            : confidence === "low"
              ? "bg-muted-soft/60"
              : "bg-accent/45"
        }`}
      />
      {editing ? (
        <Input
          ref={inputRef}
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={commit}
          onKeyDown={(event) => {
            if (event.key === "Enter") commit();
            if (event.key === "Escape") {
              // Keep the dialog open; this Escape only cancels the edit.
              event.preventDefault();
              setDraft(note.text);
              setEditing(false);
            }
          }}
          aria-label={t("memoryManager.editNote")}
          variant="compact"
          className="min-w-0 flex-1"
        />
      ) : (
        <button
          type="button"
          onClick={() => setEditing(true)}
          className="min-w-0 flex-1 cursor-text text-start text-sm leading-snug text-ink"
        >
          {note.text}
        </button>
      )}
      <span className="hidden shrink-0 text-xs text-muted sm:inline">
        {learned
          ? t("memoryManager.source.learned")
          : t("memoryManager.source.added")}
      </span>
      <span className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
        <button
          type="button"
          onClick={() => setEditing(true)}
          aria-label={t("memoryManager.editNote")}
          title={t("memoryManager.editNote")}
          className="grid h-7 w-7 cursor-pointer place-items-center rounded-md text-muted transition-colors hover:bg-ink/[0.06] hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
        >
          <Pencil className="h-3.5 w-3.5" aria-hidden="true" />
        </button>
        <button
          type="button"
          onClick={() => onDelete(note.id)}
          aria-label={t("settings.personalMemory.notes.delete")}
          title={t("settings.personalMemory.notes.delete")}
          className="grid h-7 w-7 cursor-pointer place-items-center rounded-md text-muted transition-colors hover:bg-error/10 hover:text-error focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
        >
          <Trash2 className="h-3.5 w-3.5" aria-hidden="true" />
        </button>
      </span>
    </li>
  );
};

/**
 * Everything the assistant remembers, readable at a glance and editable in
 * place.
 *
 * The old screen collapsed "About you" and the notes behind chevrons, then
 * showed every note as its own text field with an uppercase caption — so
 * reading your own memory meant opening two sections and scanning a column of
 * input boxes. Now the summary is plain text with an Edit button, the notes are
 * a list of plain lines you can search and filter, and a note becomes a field
 * only while you are editing it.
 */
export const MemorySettings: React.FC = () => {
  const { t } = useTranslation();
  const { settings, refreshSettings } = useSettings();

  const enabled = settings?.assistant_memory_enabled ?? false;
  const incognito = settings?.assistant_memory_incognito ?? false;
  const detail = (settings?.assistant_memory_detail ??
    "balanced") as MemoryDetail;
  const memory = settings?.assistant_memory;
  const notes = useMemo(() => memory?.notes ?? [], [memory?.notes]);

  const [aboutDraft, setAboutDraft] = useState("");
  const [editingAbout, setEditingAbout] = useState(false);
  const [newNote, setNewNote] = useState("");
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<NoteFilter>("all");
  const [error, setError] = useState<string | null>(null);
  const [distilling, setDistilling] = useState(false);
  const [confirmWipe, setConfirmWipe] = useState(false);

  useEffect(() => {
    if (!editingAbout) setAboutDraft(memory?.about_you ?? "");
  }, [memory?.about_you, editingAbout]);

  /** Run a command that returns a Result, surface errors, then refresh. */
  const run = useCallback(
    async (
      action: () => Promise<{ status: "ok" | "error"; error?: string }>,
    ): Promise<boolean> => {
      try {
        const res = await action();
        if (res.status === "error") {
          setError(res.error ?? t("common.saveFailed"));
          return false;
        }
        setError(null);
        return true;
      } catch (err) {
        setError(String(err));
        return false;
      } finally {
        await refreshSettings();
      }
    },
    [refreshSettings, t],
  );

  const saveAbout = async () => {
    const next = aboutDraft.trim();
    if (next !== (memory?.about_you ?? "").trim()) {
      await run(() => commands.setAssistantMemoryAboutYou(next));
    }
    setEditingAbout(false);
  };

  const addNote = async () => {
    const text = newNote.trim();
    if (!text) return;
    if (await run(() => commands.addAssistantMemoryNote(text))) setNewNote("");
  };

  const saveNote = (note: MemoryNote, text: string) =>
    void run(() => commands.updateAssistantMemoryNote(note.id, text.trim()));

  const deleteNote = (id: string) =>
    void run(() => commands.deleteAssistantMemoryNote(id));

  const distillNow = async () => {
    setDistilling(true);
    await run(() => commands.assistantDistillMemoryNow());
    setDistilling(false);
  };

  const exportMemory = async () => {
    try {
      const path = await save({
        defaultPath: "speakoflow-memory.json",
        filters: [{ name: "Memory", extensions: ["json"] }],
      });
      if (path) await run(() => commands.exportAssistantMemory(path));
    } catch (err) {
      setError(String(err));
    }
  };

  const importMemory = async () => {
    try {
      const path = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "Memory", extensions: ["json"] }],
      });
      if (typeof path === "string") {
        await run(() => commands.importAssistantMemory(path));
      }
    } catch (err) {
      setError(String(err));
    }
  };

  const wipe = async () => {
    if (await run(() => commands.clearAssistantMemory())) setConfirmWipe(false);
  };

  const visibleNotes = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return notes
      .filter((note) =>
        filter === "all"
          ? true
          : filter === "auto"
            ? note.source === "auto"
            : note.source !== "auto",
      )
      .filter((note) => !needle || note.text.toLowerCase().includes(needle))
      .sort((a, b) => (b.updated ?? "").localeCompare(a.updated ?? ""));
  }, [notes, query, filter]);

  if (!settings) return null;

  const about = (memory?.about_you ?? "").trim();
  const learnedCount = notes.filter((note) => note.source === "auto").length;

  return (
    <div className="space-y-6">
      {/* The two switches, as plain rows with their explanation in an (i). */}
      <SettingsGroup>
        <SettingContainer
          title={t("settings.personalMemory.enable.label")}
          description={t("memoryManager.tips.enable")}
          grouped
        >
          <Switch
            checked={enabled}
            onChange={(value) =>
              void run(() => commands.setAssistantMemoryEnabled(value))
            }
            label={t("settings.personalMemory.enable.label")}
          />
        </SettingContainer>
        <SettingContainer
          title={t("settings.personalMemory.incognito.label")}
          description={t("memoryManager.tips.incognito")}
          grouped
        >
          <Switch
            checked={incognito}
            onChange={(value) =>
              void run(() => commands.setAssistantMemoryIncognito(value))
            }
            label={t("settings.personalMemory.incognito.label")}
          />
        </SettingContainer>
      </SettingsGroup>

      {/* About you: plain text, edited in place. */}
      <section className="rounded-2xl border border-hairline bg-surface">
        <header className="flex items-center gap-3 border-b border-hairline px-5 py-3">
          <h3 className="flex min-w-0 flex-1 items-center gap-1 text-sm font-semibold text-ink">
            {t("settings.personalMemory.aboutYou.label")}
            <InfoTip text={t("memoryManager.tips.aboutYou")} />
          </h3>
          {!editingAbout && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setEditingAbout(true)}
            >
              <Pencil className="h-3.5 w-3.5" aria-hidden="true" />
              {t("common.edit")}
            </Button>
          )}
        </header>
        <div className="px-5 py-4">
          {editingAbout ? (
            <div className="space-y-3">
              <Textarea
                value={aboutDraft}
                onChange={(event) => setAboutDraft(event.target.value)}
                rows={4}
                autoFocus
                placeholder={t("settings.personalMemory.aboutYou.placeholder")}
                className="w-full"
              />
              <div className="flex justify-end gap-2">
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    setAboutDraft(memory?.about_you ?? "");
                    setEditingAbout(false);
                  }}
                >
                  {t("common.cancel")}
                </Button>
                <Button size="sm" onClick={() => void saveAbout()}>
                  <Check className="h-3.5 w-3.5" aria-hidden="true" />
                  {t("common.save")}
                </Button>
              </div>
            </div>
          ) : (
            <p
              className={`text-sm leading-relaxed whitespace-pre-wrap ${about ? "text-body" : "text-muted"}`}
            >
              {about || t("settings.personalMemory.aboutYou.empty")}
            </p>
          )}
        </div>
      </section>

      {/* Notes: a searchable list of plain lines. */}
      <section className="rounded-2xl border border-hairline bg-surface">
        <header className="flex flex-wrap items-center gap-3 border-b border-hairline px-5 py-3">
          <h3 className="flex items-center gap-2 text-sm font-semibold text-ink">
            {t("settings.personalMemory.notes.label")}
            <span className="rounded-full bg-surface-strong px-2 py-0.5 text-xs font-medium text-muted tabular-nums">
              {notes.length}
            </span>
          </h3>
          <div className="ms-auto flex flex-wrap items-center gap-2">
            <Segmented
              size="sm"
              label={t("memoryManager.filterLabel")}
              value={filter}
              onChange={setFilter}
              options={[
                { value: "all", label: t("memoryManager.filters.all") },
                {
                  value: "auto",
                  label: t("memoryManager.filters.learned", {
                    count: learnedCount,
                  }),
                },
                {
                  value: "user",
                  label: t("memoryManager.filters.added", {
                    count: notes.length - learnedCount,
                  }),
                },
              ]}
            />
            <div className="relative">
              <Search
                className="pointer-events-none absolute start-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-soft"
                aria-hidden="true"
              />
              <input
                type="search"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder={t("memoryManager.search")}
                aria-label={t("memoryManager.search")}
                className="h-8 w-44 rounded-lg border border-hairline-strong bg-surface ps-8 pe-7 text-[0.8125rem] text-ink placeholder:text-muted-soft focus:border-accent focus:outline-none focus:ring-2 focus:ring-accent/20"
              />
              {query && (
                <button
                  type="button"
                  onClick={() => setQuery("")}
                  aria-label={t("settings.models.clearSearch")}
                  className="absolute end-1.5 top-1/2 grid h-5 w-5 -translate-y-1/2 cursor-pointer place-items-center rounded text-muted-soft hover:text-ink"
                >
                  <X className="h-3 w-3" aria-hidden="true" />
                </button>
              )}
            </div>
          </div>
        </header>

        <div className="flex items-center gap-2 border-b border-hairline px-4 py-2.5">
          <Plus className="h-4 w-4 shrink-0 text-muted" aria-hidden="true" />
          <input
            type="text"
            value={newNote}
            onChange={(event) => setNewNote(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                void addNote();
              }
            }}
            placeholder={t("settings.personalMemory.notes.addPlaceholder")}
            aria-label={t("settings.personalMemory.notes.addPlaceholder")}
            className="min-w-0 flex-1 bg-transparent py-1 text-sm text-ink placeholder:text-muted-soft focus:outline-none"
          />
          {newNote.trim() && (
            <Button size="sm" onClick={() => void addNote()}>
              {t("settings.personalMemory.notes.add")}
            </Button>
          )}
        </div>

        {notes.length === 0 ? (
          <p className="px-5 py-8 text-center text-sm text-muted">
            {t("settings.personalMemory.notes.empty")}
          </p>
        ) : visibleNotes.length === 0 ? (
          <p className="px-5 py-8 text-center text-sm text-muted">
            {t("memoryManager.noMatches")}
          </p>
        ) : (
          <ul className="max-h-[22rem] divide-y divide-hairline overflow-y-auto">
            {visibleNotes.map((note) => (
              <NoteRow
                key={note.id}
                note={note}
                onSave={saveNote}
                onDelete={deleteNote}
              />
            ))}
          </ul>
        )}
      </section>

      {/* How much of it each reply uses. */}
      <section className="flex flex-wrap items-center justify-between gap-x-6 gap-y-3 rounded-2xl border border-hairline bg-surface px-5 py-4">
        <div className="min-w-0 max-w-sm">
          <h3 className="text-sm font-semibold text-ink">
            {t("settings.personalMemory.detail.label")}
          </h3>
          <p className="mt-0.5 text-xs leading-snug text-muted">
            {t(`memoryManager.detailHints.${detail}`)}
          </p>
        </div>
        <Segmented
          label={t("settings.personalMemory.detail.label")}
          value={detail}
          onChange={(value) =>
            void run(() => commands.setAssistantMemoryDetail(value))
          }
          options={(["light", "balanced", "detailed"] as const).map(
            (value) => ({
              value,
              label: t(`settings.personalMemory.detail.options.${value}`),
            }),
          )}
        />
      </section>

      {error && (
        <p role="alert" className="text-sm text-error">
          {error}
        </p>
      )}

      {/* Toolbar: learn now, move it, or erase it. */}
      <footer className="flex flex-wrap items-center gap-2 border-t border-hairline pt-4">
        <Button
          variant="secondary"
          size="sm"
          onClick={() => void distillNow()}
          disabled={distilling || !enabled}
          title={t("settings.personalMemory.distill.description")}
        >
          {distilling ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
          ) : (
            <Sparkles className="h-3.5 w-3.5" aria-hidden="true" />
          )}
          {t("memoryManager.learnNow")}
        </Button>
        <Button variant="ghost" size="sm" onClick={() => void exportMemory()}>
          <Download className="h-3.5 w-3.5" aria-hidden="true" />
          {t("settings.personalMemory.export")}
        </Button>
        <Button variant="ghost" size="sm" onClick={() => void importMemory()}>
          <Upload className="h-3.5 w-3.5" aria-hidden="true" />
          {t("settings.personalMemory.import")}
        </Button>
        <div className="ms-auto flex items-center gap-2">
          {confirmWipe ? (
            <>
              <span className="text-xs text-muted">
                {t("settings.personalMemory.wipe.confirm")}
              </span>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setConfirmWipe(false)}
              >
                {t("settings.personalMemory.wipe.cancel")}
              </Button>
              <Button variant="danger" size="sm" onClick={() => void wipe()}>
                {t("settings.personalMemory.wipe.yes")}
              </Button>
            </>
          ) : (
            <Button
              variant="danger-ghost"
              size="sm"
              onClick={() => setConfirmWipe(true)}
            >
              <Trash2 className="h-3.5 w-3.5" aria-hidden="true" />
              {t("settings.personalMemory.wipe.button")}
            </Button>
          )}
        </div>
      </footer>
    </div>
  );
};

export default MemorySettings;
