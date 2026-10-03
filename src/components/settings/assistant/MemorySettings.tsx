import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { open, save } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import {
  Check,
  MoreHorizontal,
  Pencil,
  Plus,
  Search,
  Trash2,
  X,
} from "lucide-react";
import {
  commands,
  type MemoryDetail,
  type MemoryNote,
  type Result,
} from "@/bindings";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Textarea } from "@/components/ui/Textarea";
import { InfoTip } from "@/components/ui/InfoTip";
import { Dialog } from "@/components/ui/Dialog";
import { MenuButton } from "@/components/ui/Menu";
import { Segmented } from "@/components/ui/Segmented";
import { useSettings } from "@/hooks/useSettings";
import { useSettingCommand } from "@/hooks/useSettingCommand";

/**
 * What the assistant remembers, in a window of its own.
 *
 * The page keeps one quiet row — the Memory switch and a Manage button, the
 * same shape as Profiles beside it — and everything memory holds opens in a
 * dialog: how much of it goes into a reply, the summary it keeps of you, and
 * the things it has picked up, each editable where it stands. Rendering all of
 * that under the switch turned the Assistant page into a memory page. The rarer
 * actions (learn now, export, import, erase) live in the dialog's ⋯ menu.
 */

/** A search box only earns its place once the list is long. */
const SEARCH_FROM = 8;

const MEMORY_DETAILS: MemoryDetail[] = ["light", "balanced", "detailed"];

/**
 * Run a memory command, surface its own error (the backend explains *why* —
 * "that looks like a secret" — which a generic "couldn't save" would hide),
 * then refresh. Resolves to whether it worked.
 */
const useMemoryCommand = () => {
  const { t } = useTranslation();
  const { refreshSettings } = useSettings();
  return useCallback(
    async (command: Promise<Result<unknown, string>>): Promise<boolean> => {
      try {
        const result = await command;
        if (result.status === "error") {
          toast.error(result.error || t("common.saveFailed"));
          return false;
        }
        return true;
      } catch (error) {
        console.error("Memory command failed:", error);
        toast.error(t("common.saveFailed"));
        return false;
      } finally {
        await refreshSettings();
      }
    },
    [refreshSettings, t],
  );
};

/** A note as a plain line; click it (or the pencil) to edit it in place. */
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

  return (
    <li className="group flex min-h-11 items-center gap-3 px-3.5 py-1.5 transition-colors hover:bg-surface-muted">
      {editing ? (
        <Input
          ref={inputRef}
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={commit}
          onKeyDown={(event) => {
            if (event.key === "Enter") commit();
            if (event.key === "Escape") {
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
          className="min-w-0 flex-1 cursor-text py-1 text-start text-sm leading-snug text-ink"
        >
          {note.text}
        </button>
      )}
      <span className="hidden shrink-0 text-xs text-muted sm:inline">
        {note.source === "auto"
          ? t("memoryManager.source.learned")
          : t("memoryManager.source.added")}
      </span>
      <span className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100">
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

/** A small heading inside the memory area, with its short (i). */
const Heading: React.FC<{
  title: string;
  tip: string;
  children?: React.ReactNode;
}> = ({ title, tip, children }) => (
  <div className="flex min-h-8 items-center gap-2">
    <h4 className="flex items-center gap-1 text-[0.8125rem] font-medium text-ink">
      {title}
      <InfoTip text={tip} />
    </h4>
    {children}
  </div>
);

/** The summary it keeps of you, edited in place. */
const AboutYou: React.FC = () => {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const run = useMemoryCommand();
  const stored = settings?.assistant_memory?.about_you ?? "";
  const about = stored.trim();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(stored);

  useEffect(() => {
    if (!editing) setDraft(stored);
  }, [stored, editing]);

  const save = async () => {
    if (draft.trim() === about) {
      setEditing(false);
      return;
    }
    if (await run(commands.setAssistantMemoryAboutYou(draft.trim()))) {
      setEditing(false);
    }
  };

  return (
    <section>
      <Heading
        title={t("settings.personalMemory.aboutYou.label")}
        tip={t("assistantPage.memory.aboutTip")}
      >
        {!editing && (
          <Button
            variant="ghost"
            size="sm"
            className="ms-auto"
            onClick={() => setEditing(true)}
          >
            <Pencil className="h-3.5 w-3.5" aria-hidden="true" />
            {about ? t("common.edit") : t("assistantPage.memory.write")}
          </Button>
        )}
      </Heading>
      {editing ? (
        <div className="mt-1.5 space-y-2.5">
          <Textarea
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                // Cancel the edit, not the page.
                event.preventDefault();
                setEditing(false);
              }
            }}
            rows={4}
            autoFocus
            placeholder={t("settings.personalMemory.aboutYou.placeholder")}
            aria-label={t("settings.personalMemory.aboutYou.label")}
            className="w-full"
          />
          <div className="flex justify-end gap-2">
            <Button variant="ghost" size="sm" onClick={() => setEditing(false)}>
              {t("common.cancel")}
            </Button>
            <Button size="sm" onClick={() => void save()}>
              <Check className="h-3.5 w-3.5" aria-hidden="true" />
              {t("common.save")}
            </Button>
          </div>
        </div>
      ) : (
        <p
          className={`mt-1 whitespace-pre-wrap text-sm leading-relaxed ${about ? "text-body" : "text-muted"}`}
        >
          {about || t("assistantPage.memory.aboutEmpty")}
        </p>
      )}
    </section>
  );
};

/** The things it has picked up, plus anything added by hand. */
const Notes: React.FC = () => {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const run = useMemoryCommand();
  const notes = useMemo(
    () => settings?.assistant_memory?.notes ?? [],
    [settings?.assistant_memory?.notes],
  );
  const [newNote, setNewNote] = useState("");
  const [query, setQuery] = useState("");

  const sorted = useMemo(
    () =>
      [...notes].sort((a, b) =>
        (b.updated ?? "").localeCompare(a.updated ?? ""),
      ),
    [notes],
  );
  const needle = query.trim().toLowerCase();
  // The dialog scrolls, so every note is listed; a search narrows it.
  const shown = needle
    ? sorted.filter((note) => note.text.toLowerCase().includes(needle))
    : sorted;

  const add = async () => {
    const text = newNote.trim();
    if (!text) return;
    if (await run(commands.addAssistantMemoryNote(text))) setNewNote("");
  };

  return (
    <section>
      <Heading
        title={t("assistantPage.memory.notesTitle")}
        tip={t("assistantPage.memory.notesTip")}
      >
        {notes.length > 0 && (
          <span className="rounded-full bg-surface-strong px-1.5 py-px text-[11px] font-medium text-muted tabular-nums">
            {notes.length}
          </span>
        )}
        {notes.length >= SEARCH_FROM && (
          <div className="relative ms-auto">
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
        )}
      </Heading>

      <div className="mt-1.5 overflow-hidden rounded-xl border border-hairline">
        <div className="flex items-center gap-2.5 px-3.5 py-1.5">
          <Plus className="h-4 w-4 shrink-0 text-muted" aria-hidden="true" />
          <input
            type="text"
            value={newNote}
            onChange={(event) => setNewNote(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                void add();
              }
            }}
            placeholder={t("settings.personalMemory.notes.addPlaceholder")}
            aria-label={t("settings.personalMemory.notes.addPlaceholder")}
            className="h-8 min-w-0 flex-1 bg-transparent text-sm text-ink placeholder:text-muted-soft focus:outline-none"
          />
          {newNote.trim() && (
            <Button size="sm" onClick={() => void add()}>
              {t("settings.personalMemory.notes.add")}
            </Button>
          )}
        </div>
        {notes.length === 0 ? (
          <p className="border-t border-hairline px-3.5 py-4 text-sm text-muted">
            {t("assistantPage.memory.notesEmpty")}
          </p>
        ) : shown.length === 0 ? (
          <p className="border-t border-hairline px-3.5 py-4 text-sm text-muted">
            {t("memoryManager.noMatches")}
          </p>
        ) : (
          <ul className="divide-y divide-hairline border-t border-hairline">
            {shown.map((note) => (
              <NoteRow
                key={note.id}
                note={note}
                onSave={(target, text) =>
                  void run(
                    commands.updateAssistantMemoryNote(target.id, text.trim()),
                  )
                }
                onDelete={(id) =>
                  void run(commands.deleteAssistantMemoryNote(id))
                }
              />
            ))}
          </ul>
        )}
      </div>
    </section>
  );
};

/** How much of what it remembers goes into each reply. */
const DetailRow: React.FC = () => {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const run = useSettingCommand();
  const detail = settings?.assistant_memory_detail ?? "balanced";
  return (
    <Heading
      title={t("settings.personalMemory.detail.label")}
      tip={t("assistantPage.memory.detailTip")}
    >
      <div className="ms-auto flex items-center gap-1.5">
        <Segmented
          label={t("settings.personalMemory.detail.label")}
          value={detail}
          onChange={(value: MemoryDetail) =>
            void run(commands.setAssistantMemoryDetail(value))
          }
          options={MEMORY_DETAILS.map((value) => ({
            value,
            label: t(`settings.personalMemory.detail.options.${value}`),
          }))}
        />
        <MemoryActions />
      </div>
    </Heading>
  );
};

/** Everything it remembers, for the Memory dialog. */
export const MemoryManager: React.FC = () => (
  <div className="space-y-6">
    <DetailRow />
    <AboutYou />
    <Notes />
  </div>
);

/**
 * The ⋯ in the Memory dialog: the actions worth having but not worth a
 * button each — learn from the chat now, move memory to another computer, or
 * erase it (which asks first).
 */
export const MemoryActions: React.FC = () => {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const run = useMemoryCommand();
  const [confirmErase, setConfirmErase] = useState(false);
  const [erasing, setErasing] = useState(false);
  const noteCount = settings?.assistant_memory?.notes?.length ?? 0;

  const learnNow = async () => {
    if (await run(commands.assistantDistillMemoryNow())) {
      toast.success(t("assistantPage.memory.learnedToast"));
    }
  };

  const exportMemory = async () => {
    try {
      const path = await save({
        defaultPath: "speakoflow-memory.json",
        filters: [{ name: "Memory", extensions: ["json"] }],
      });
      if (path && (await run(commands.exportAssistantMemory(path)))) {
        toast.success(t("assistantPage.memory.exportedToast"));
      }
    } catch (error) {
      toast.error(String(error));
    }
  };

  const importMemory = async () => {
    try {
      const path = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "Memory", extensions: ["json"] }],
      });
      if (
        typeof path === "string" &&
        (await run(commands.importAssistantMemory(path)))
      ) {
        toast.success(t("assistantPage.memory.importedToast"));
      }
    } catch (error) {
      toast.error(String(error));
    }
  };

  const erase = async () => {
    setErasing(true);
    if (await run(commands.clearAssistantMemory())) setConfirmErase(false);
    setErasing(false);
  };

  return (
    <>
      <MenuButton
        ariaLabel={t("assistantPage.memory.more")}
        title={t("assistantPage.memory.more")}
        width={230}
        className="grid h-8 w-8 cursor-pointer place-items-center rounded-lg text-muted transition-colors hover:bg-ink/[0.05] hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
        items={[
          {
            id: "learn",
            label: t("assistantPage.memory.learnNow"),
            onSelect: () => void learnNow(),
          },
          {
            id: "export",
            label: t("assistantPage.memory.export"),
            onSelect: () => void exportMemory(),
          },
          {
            id: "import",
            label: t("assistantPage.memory.import"),
            onSelect: () => void importMemory(),
          },
          {
            id: "erase",
            label: t("assistantPage.memory.erase"),
            tone: "danger",
            separated: true,
            onSelect: () => setConfirmErase(true),
          },
        ]}
      >
        <MoreHorizontal className="h-4 w-4" aria-hidden="true" />
      </MenuButton>
      <Dialog
        open={confirmErase}
        onClose={() => setConfirmErase(false)}
        size="sm"
        title={t("assistantPage.memory.eraseTitle")}
        footer={
          <>
            <Button
              variant="ghost"
              size="sm"
              className="ms-auto"
              onClick={() => setConfirmErase(false)}
            >
              {t("common.cancel")}
            </Button>
            <Button
              variant="danger"
              size="sm"
              disabled={erasing}
              onClick={() => void erase()}
            >
              {t("assistantPage.memory.eraseConfirm")}
            </Button>
          </>
        }
      >
        <p className="text-sm leading-relaxed text-body">
          {t("assistantPage.memory.eraseBody", { count: noteCount })}
        </p>
      </Dialog>
    </>
  );
};
