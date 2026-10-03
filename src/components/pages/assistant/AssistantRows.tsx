import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { Check, ChevronDown, ChevronUp, Loader2, X } from "lucide-react";
import {
  commands,
  type AssistantCharacter,
  type AssistantResponseLength,
  type AssistantSearchDepth,
  type Reminder,
  type VisionCaptureTiming,
} from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { useSettingCommand } from "@/hooks/useSettingCommand";
import { SettingContainer } from "@/components/ui/SettingContainer";
import { Segmented } from "@/components/ui/Segmented";
import { LogoChoice } from "@/components/ui/LogoChoice";
import { LogoSelect } from "@/components/ui/LogoSelect";
import { Switch } from "@/components/ui/Switch";
import { Button } from "@/components/ui/Button";
import { Dialog } from "@/components/ui/Dialog";
import { Input } from "@/components/ui/Input";
import { ProviderTile } from "@/components/icons/ProviderLogos";
import ScreenRecordingPermission from "@/components/ScreenRecordingPermission";
import { MemoryManager } from "@/components/settings/assistant/MemorySettings";

/**
 * The Assistant page's settings, as plain rows: a name, a short (i), and the
 * control. Anything more — a list a feature manages, a picker that only
 * matters once it is on — appears under its row only when the feature is on,
 * so a switched-off feature is one quiet line.
 */

/* ─────────────────────────────── vision ─────────────────────────────── */

/**
 * Two switches, one per surface. On means the assistant *may* look — it
 * decides per question and looks only when the question is about the screen —
 * not that every question captures. Off by default: a screenshot goes to the
 * provider the user picked, so seeing the screen is something they turn on.
 *
 * Capture timing only changes a spoken quick ask (a call captures on demand),
 * so it appears under the quick-ask switch and nowhere else.
 */
export const VisionRows: React.FC = () => {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const run = useSettingCommand();
  const ask = settings?.assistant_ask_screen_access ?? false;
  const call = settings?.assistant_call_screen_access ?? false;
  const timing = settings?.assistant_vision_capture_timing ?? "immediate";

  return (
    <>
      <SettingContainer
        title={t("assistantPage.rows.screen.ask")}
        description={t("assistantPage.tips.screenAsk")}
        grouped
      >
        <Switch
          checked={ask}
          onChange={(value) =>
            void run(commands.setAssistantAskScreenAccess(value))
          }
          label={t("assistantPage.rows.screen.ask")}
        />
      </SettingContainer>
      {ask && (
        <SettingContainer
          title={t("settings.assistant.vision.timing.label")}
          description={t("assistantPage.tips.captureTiming")}
          grouped
        >
          <Segmented
            label={t("settings.assistant.vision.timing.label")}
            value={timing}
            onChange={(next: VisionCaptureTiming) =>
              void run(commands.setAssistantVisionCaptureTiming(next))
            }
            options={[
              {
                value: "immediate",
                label: t("assistantPage.cards.vision.timing.immediate"),
              },
              {
                value: "on_send",
                label: t("assistantPage.cards.vision.timing.onSend"),
              },
            ]}
          />
        </SettingContainer>
      )}
      <SettingContainer
        title={t("assistantPage.rows.screen.call")}
        description={t("assistantPage.tips.screenCall")}
        grouped
      >
        <Switch
          checked={call}
          onChange={(value) =>
            void run(commands.setAssistantCallScreenAccess(value))
          }
          label={t("assistantPage.rows.screen.call")}
        />
      </SettingContainer>
      {/* macOS only; renders nothing elsewhere or once granted. */}
      {(ask || call) && <ScreenRecordingPermission />}
    </>
  );
};

/* ───────────────────────────── web search ───────────────────────────── */

const SEARCH_PROVIDERS = [
  "tinyfish",
  "serper",
  "brave",
  "tavily",
  "exa",
  "serpapi",
] as const;

/** Where to get a key, for the one line under the key field. */
const KEY_SITES: Record<string, string> = {
  serper: "serper.dev",
  brave: "brave.com/search/api",
  tavily: "tavily.com",
  exa: "exa.ai",
  serpapi: "serpapi.com",
  tinyfish: "tinyfish.ai",
};

/**
 * Which search company, by its logo, and its key. Shown once search is on.
 *
 * Folded to one line — the provider in use and whether its key is saved —
 * once it works, because a six-tile picker and a key field open all the time
 * made the longest list on the page longer for a choice made once. It opens by
 * itself when it needs something: the selected provider has no key yet.
 */
const WebSearchSetup: React.FC = () => {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const run = useSettingCommand();
  const provider = settings?.assistant_web_search_provider ?? "serper";
  const keys = settings?.web_search_api_keys ?? {};
  const storedKey = keys[provider] ?? "";
  const [draft, setDraft] = useState(storedKey);
  const [testing, setTesting] = useState(false);
  // Decided once, on mount: open only if there is something to do. Picking a
  // different provider while open must not snap it shut under the cursor.
  const [expanded, setExpanded] = useState(() => !storedKey.trim());

  useEffect(() => setDraft(storedKey), [storedKey, provider]);

  const providerLabel = (id: string) =>
    t(`settings.assistant.webSearch.providers.${id}`).replace(
      /\s*\([^)]*\)\s*$/,
      "",
    );

  const saveKey = async () => {
    const next = draft.trim();
    if (next === storedKey) return;
    await run(commands.setAssistantWebSearchApiKey(provider, next));
  };

  const test = async () => {
    setTesting(true);
    try {
      await saveKey();
      const result = await commands.assistantTestWebSearch(
        "who is the prime minister of canada",
      );
      if (result.status === "ok") {
        toast.success(
          t("settings.assistant.webSearch.testResult", {
            count: result.data.length,
          }),
        );
      } else {
        toast.error(result.error);
      }
    } catch (error) {
      toast.error(String(error));
    } finally {
      setTesting(false);
    }
  };

  const keyStatus = storedKey ? (
    <span className="inline-flex items-center gap-1 text-success">
      <Check className="h-3.5 w-3.5" aria-hidden="true" />
      {t("assistantPage.cards.keySaved")}
    </span>
  ) : (
    t("assistantPage.cards.webSearch.getKey", {
      site: KEY_SITES[provider] ?? "",
    })
  );

  if (!expanded) {
    return (
      <div className="flex items-center gap-3">
        <ProviderTile id={provider} kind="search" size="sm" />
        <div className="min-w-0 flex-1">
          <div className="truncate text-[0.8125rem] font-medium text-ink">
            {providerLabel(provider)}
          </div>
          <div className="truncate text-xs text-muted">{keyStatus}</div>
        </div>
        <Button
          variant="secondary"
          size="sm"
          onClick={() => setExpanded(true)}
          aria-expanded={false}
        >
          {t("common.change")}
          <ChevronDown className="h-3.5 w-3.5" aria-hidden="true" />
        </Button>
      </div>
    );
  }

  return (
    <div>
      <LogoChoice
        label={t("settings.assistant.webSearch.providerLabel")}
        value={provider}
        onChange={(next) =>
          void run(commands.setAssistantWebSearchProvider(next))
        }
        // Six providers: wide enough tiles that they sit 3 + 3 (or 2 + 2 + 2
        // on a narrow window), never 5 + 1.
        minTile="15rem"
        readyLabel={t("assistantPage.cards.keySaved")}
        options={SEARCH_PROVIDERS.map((id) => ({
          value: id,
          label: providerLabel(id),
          hint: t(`assistantPage.cards.webSearch.hints.${id}`),
          icon: <ProviderTile id={id} kind="search" size="md" />,
          ready: !!keys[id]?.trim(),
        }))}
      />
      <div className="mt-3 flex gap-2">
        <Input
          type="password"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={() => void saveKey()}
          placeholder={t("assistantPage.cards.webSearch.keyFor", {
            provider: providerLabel(provider),
          })}
          aria-label={t("assistantPage.cards.webSearch.keyFor", {
            provider: providerLabel(provider),
          })}
          variant="compact"
          className="min-w-0 flex-1"
        />
        <Button
          variant="secondary"
          size="sm"
          onClick={() => void test()}
          disabled={testing || !draft.trim()}
          className="h-auto"
        >
          {testing && (
            <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
          )}
          {t("settings.assistant.webSearch.testButton")}
        </Button>
      </div>
      <div className="mt-1.5 flex items-center justify-between gap-3">
        <p className="text-xs text-muted">{keyStatus}</p>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => setExpanded(false)}
          aria-expanded={true}
          className="-me-2"
        >
          {t("common.showLess")}
          <ChevronUp className="h-3.5 w-3.5" aria-hidden="true" />
        </Button>
      </div>
    </div>
  );
};

export const WebSearchRows: React.FC = () => {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const run = useSettingCommand();
  const enabled = settings?.assistant_web_search_enabled ?? false;
  const assistantProvider = settings?.assistant_provider_id ?? "";

  return (
    <>
      <SettingContainer
        title={t("settings.assistant.webSearch.title")}
        description={t("assistantPage.tips.webSearch")}
        grouped
        details={enabled ? <WebSearchSetup /> : undefined}
      >
        <Switch
          checked={enabled}
          onChange={(value) =>
            void run(commands.setAssistantWebSearchEnabled(value))
          }
          label={t("settings.assistant.webSearch.enableLabel")}
        />
      </SettingContainer>
      {enabled && (
        <SettingContainer
          title={t("settings.assistant.webSearch.depthLabel")}
          description={t("assistantPage.tips.depth")}
          grouped
        >
          <Segmented
            label={t("settings.assistant.webSearch.depthLabel")}
            value={settings?.assistant_search_depth ?? "medium"}
            onChange={(depth: AssistantSearchDepth) =>
              void run(commands.setAssistantSearchDepth(depth))
            }
            options={(["low", "medium", "high"] as const).map((value) => ({
              value,
              label: t(`settings.assistant.webSearch.depthOptions.${value}`),
            }))}
          />
        </SettingContainer>
      )}
      {enabled && assistantProvider === "openrouter" && (
        <SettingContainer
          title={t("settings.assistant.webSearch.openRouterNativeLabel")}
          description={t("assistantPage.tips.openRouterSearch")}
          grouped
        >
          <Switch
            checked={settings?.assistant_prefer_provider_web_search ?? true}
            onChange={(value) =>
              void run(commands.setAssistantPreferProviderWebSearch(value))
            }
            label={t("settings.assistant.webSearch.openRouterNativeLabel")}
          />
        </SettingContainer>
      )}
      {enabled && assistantProvider === "builtin" && (
        <SettingContainer
          title={t("settings.assistant.webSearch.localSmartLabel")}
          description={t("assistantPage.tips.localSmart")}
          grouped
        >
          <Switch
            checked={settings?.assistant_local_search_smart ?? false}
            onChange={(value) =>
              void run(commands.setAssistantLocalSearchSmart(value))
            }
            label={t("settings.assistant.webSearch.localSmartLabel")}
          />
        </SettingContainer>
      )}
    </>
  );
};

/* ───────────────────────────── reminders ───────────────────────────── */

/** "4:58 PM" today, "Sun 3:23 PM" this week, "Oct 3" after that. */
const formatDue = (reminder: Reminder, locale: string): string => {
  const due = new Date(reminder.due_at);
  if (Number.isNaN(due.getTime())) return reminder.due_at;
  const now = new Date();
  const days = (due.getTime() - now.getTime()) / 86_400_000;
  if (due.toDateString() === now.toDateString()) {
    return due.toLocaleTimeString(locale, {
      hour: "numeric",
      minute: "2-digit",
    });
  }
  if (days < 6) {
    return due.toLocaleString(locale, {
      weekday: "short",
      hour: "numeric",
      minute: "2-digit",
    });
  }
  return due.toLocaleDateString(locale, { month: "short", day: "numeric" });
};

/**
 * Reminders are made by talking, so the row has nothing to switch: it says
 * how many are set, and lists them — each cancellable — only when there are
 * any. How to ask is in the (i).
 */
export const RemindersRow: React.FC = () => {
  const { t, i18n } = useTranslation();
  const [reminders, setReminders] = useState<Reminder[] | null>(null);

  const refresh = useCallback(() => {
    void commands
      .listReminders()
      .then((list) => setReminders(list ?? []))
      .catch(() => setReminders([]));
  }, []);

  useEffect(() => {
    refresh();
    const unlisten = listen<Reminder[]>("reminders-changed", (event) =>
      setReminders(event.payload ?? []),
    );
    return () => {
      void unlisten.then((off) => off());
    };
  }, [refresh]);

  const cancel = async (id: string) => {
    try {
      const result = await commands.completeReminder(id);
      if (result.status !== "ok") throw new Error(result.error);
    } catch (error) {
      console.error("Failed to cancel reminder:", error);
      toast.error(t("common.saveFailed"));
    }
    refresh();
  };

  const list = reminders ?? [];
  const shown = list.slice(0, 4);

  return (
    <SettingContainer
      title={t("reminders.settings.label")}
      description={t("assistantPage.tips.reminders")}
      grouped
      details={
        shown.length > 0 ? (
          <ul className="-mx-2 space-y-0.5">
            {shown.map((reminder) => (
              <li
                key={reminder.id}
                className="group flex items-center gap-3 rounded-lg px-2 py-1.5 transition-colors hover:bg-surface-muted"
              >
                <span
                  className="w-[5.75rem] shrink-0 truncate text-xs font-medium text-accent tabular-nums"
                  title={new Date(reminder.due_at).toLocaleString(
                    i18n.language,
                  )}
                >
                  {reminder.fired
                    ? t("reminders.settings.waiting")
                    : formatDue(reminder, i18n.language)}
                </span>
                <span className="min-w-0 flex-1 truncate text-[0.8125rem] text-ink">
                  {reminder.text}
                </span>
                <button
                  type="button"
                  onClick={() => void cancel(reminder.id)}
                  aria-label={t("reminders.settings.cancel")}
                  title={t("reminders.settings.cancel")}
                  className="grid h-6 w-6 shrink-0 cursor-pointer place-items-center rounded-md text-muted opacity-60 transition-[opacity,color,background-color] group-hover:opacity-100 hover:bg-error/10 hover:text-error focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
                >
                  <X className="h-3.5 w-3.5" aria-hidden="true" />
                </button>
              </li>
            ))}
            {list.length > shown.length && (
              <li className="px-2 pt-0.5 text-xs text-muted">
                {t("assistantPage.cards.reminders.more", {
                  count: list.length - shown.length,
                })}
              </li>
            )}
          </ul>
        ) : undefined
      }
    >
      <span className="text-[0.8125rem] text-muted">
        {reminders === null
          ? ""
          : list.length > 0
            ? t("assistantPage.rows.reminders.count", { count: list.length })
            : t("assistantPage.rows.reminders.none")}
      </span>
    </SettingContainer>
  );
};

/* ────────────────────────────── profiles ────────────────────────────── */

/** The active persona as a dropdown, and the way into managing them all. */
export const ProfilesRow: React.FC<{
  onManage: () => void;
  avatar: (character: AssistantCharacter, size: number) => React.ReactNode;
}> = ({ onManage, avatar }) => {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const run = useSettingCommand();
  const characters = settings?.assistant_characters ?? [];
  const activeId = settings?.assistant_active_character_id ?? "default";

  return (
    <SettingContainer
      title={t("assistantPage.cards.profiles.title")}
      description={t("assistantPage.tips.profiles")}
      grouped
    >
      <div className="flex items-center gap-2">
        <LogoSelect
          options={characters.map((character) => ({
            value: character.id,
            label: character.name,
            hint: character.description || undefined,
            icon: avatar(character, 24),
          }))}
          value={activeId}
          onChange={(id) => void run(commands.setAssistantActiveCharacter(id))}
          className="w-[14rem]"
          ariaLabel={t("assistantPage.cards.profiles.title")}
          searchable={false}
        />
        <Button variant="secondary" onClick={onManage} className="h-10">
          {t("assistantPage.rows.profiles.manage")}
        </Button>
      </div>
    </SettingContainer>
  );
};

/* ─────────────────────────────── memory ─────────────────────────────── */

/**
 * Memory is a switch and a way in, the same shape as Profiles below it. What
 * it remembers — and how much of that goes into a reply — opens in its own
 * window, so the page stays a list of settings. Manage stays offered while
 * memory is off if anything is saved, so it can still be read or erased.
 */
export const MemoryRow: React.FC = () => {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const run = useSettingCommand();
  const [open, setOpen] = useState(false);
  const enabled = settings?.assistant_memory_enabled ?? false;
  const memory = settings?.assistant_memory;
  const hasMemory =
    (memory?.notes?.length ?? 0) > 0 || Boolean(memory?.about_you?.trim());

  return (
    <>
      <SettingContainer
        title={t("assistantPage.cards.memory.title")}
        description={t("assistantPage.tips.memory")}
        grouped
      >
        <div className="flex items-center gap-3">
          {(enabled || hasMemory) && (
            <Button variant="secondary" onClick={() => setOpen(true)}>
              {t("assistantPage.rows.memory.manage")}
            </Button>
          )}
          <Switch
            checked={enabled}
            onChange={(value) =>
              void run(commands.setAssistantMemoryEnabled(value))
            }
            label={t("settings.personalMemory.enable.label")}
          />
        </div>
      </SettingContainer>
      <Dialog
        open={open}
        onClose={() => setOpen(false)}
        size="lg"
        title={t("assistantPage.cards.memory.title")}
        description={t("assistantPage.memory.dialogDescription")}
      >
        <MemoryManager />
      </Dialog>
    </>
  );
};

/* ────────────────────────────── replies ────────────────────────────── */

const LENGTHS: AssistantResponseLength[] = [
  "default",
  "short",
  "medium",
  "long",
];

export const RepliesRows: React.FC = () => {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const run = useSettingCommand();
  const length = settings?.assistant_response_length ?? "default";
  const stored = settings?.assistant_max_history_messages ?? 12;
  const [history, setHistory] = useState(String(stored));

  useEffect(() => setHistory(String(stored)), [stored]);

  const saveHistory = () => {
    const parsed = Math.max(0, Math.min(200, parseInt(history, 10) || 0));
    setHistory(String(parsed));
    if (parsed !== stored) {
      void run(commands.setAssistantMaxHistoryMessages(parsed));
    }
  };

  return (
    <>
      <SettingContainer
        title={t("assistantPage.rows.replyLength")}
        description={t("assistantPage.tips.replyLength")}
        grouped
      >
        <Segmented
          label={t("assistantPage.rows.replyLength")}
          value={length}
          onChange={(next) =>
            void run(commands.setAssistantResponseLength(next))
          }
          options={LENGTHS.map((value) => ({
            value,
            label: t(`assistantPage.cards.replies.lengths.${value}`),
          }))}
        />
      </SettingContainer>
      <SettingContainer
        title={t("assistantPage.cards.replies.history")}
        description={t("assistantPage.tips.history")}
        grouped
      >
        <Input
          type="number"
          min={0}
          max={200}
          value={history}
          onChange={(event) => setHistory(event.target.value)}
          onBlur={saveHistory}
          onKeyDown={(event) => {
            if (event.key === "Enter") event.currentTarget.blur();
          }}
          aria-label={t("assistantPage.cards.replies.history")}
          variant="compact"
          className="w-20 text-end tabular-nums"
        />
      </SettingContainer>
      <SettingContainer
        title={t("settings.assistant.autoSummarize.label")}
        description={t("assistantPage.tips.summarize")}
        grouped
      >
        <Switch
          checked={settings?.assistant_auto_summarize ?? true}
          onChange={(value) =>
            void run(commands.setAssistantAutoSummarize(value))
          }
          label={t("settings.assistant.autoSummarize.label")}
        />
      </SettingContainer>
    </>
  );
};
