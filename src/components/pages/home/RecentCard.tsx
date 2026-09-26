import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, Copy, Sparkles, Wand2 } from "lucide-react";
import { commands, events, type HistoryEntry } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { Keycaps } from "@/components/ui/Keycaps";
import { SectionTitle, TextLink } from "@/components/ui/Page";
import { useNavigation } from "@/components/shell/navigation";
import { formatRelativeTime } from "@/utils/dateFormat";

const RECENT_LIMIT = 5;

/** Stable marker written by src-tauri/src/flow.rs on Flow generations. */
const FLOW_HISTORY_MARKER = "Generate with Flow";

/** What was actually pasted: the cleaned text when cleanup changed something. */
export const finalTextOf = (entry: HistoryEntry): string => {
  const cleaned = entry.post_processed_text?.trim();
  return cleaned ? cleaned : entry.transcription_text.trim();
};

const RecentRow: React.FC<{ entry: HistoryEntry }> = ({ entry }) => {
  const { t, i18n } = useTranslation();
  const [copied, setCopied] = useState(false);
  const text = finalTextOf(entry);
  const isFlow = entry.post_process_prompt === FLOW_HISTORY_MARKER;
  const cleaned =
    !isFlow &&
    !!entry.post_processed_text?.trim() &&
    entry.post_processed_text.trim() !== entry.transcription_text.trim();

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch (error) {
      console.error("Failed to copy:", error);
    }
  };

  return (
    <li className="group flex items-start gap-3 px-5 py-3.5">
      <div className="min-w-0 flex-1">
        <p className="line-clamp-2 select-text text-[0.9375rem] leading-relaxed text-ink">
          {text}
        </p>
        <p className="mt-1 flex items-center text-xs text-muted">
          {formatRelativeTime(String(entry.timestamp), i18n.language)}
          {(cleaned || isFlow) && (
            <>
              <span aria-hidden="true" className="mx-1.5 text-muted-soft">
                ·
              </span>
              {/* An icon rather than a caption, as on the History page. */}
              <span
                role="img"
                aria-label={
                  isFlow
                    ? t("settings.history.flowLabel")
                    : t("home.recent.cleaned")
                }
                title={
                  isFlow
                    ? t("settings.history.flowLabel")
                    : t("home.recent.cleaned")
                }
                className="inline-flex text-accent"
              >
                {isFlow ? (
                  <Sparkles className="h-3 w-3" aria-hidden="true" />
                ) : (
                  <Wand2 className="h-3 w-3" aria-hidden="true" />
                )}
              </span>
            </>
          )}
        </p>
      </div>
      <button
        type="button"
        onClick={() => void copy()}
        aria-label={t("home.recent.copy")}
        title={t("home.recent.copy")}
        className="grid h-8 w-8 shrink-0 cursor-pointer place-items-center rounded-lg text-muted opacity-0 transition-[opacity,background-color,color] group-hover:opacity-100 hover:bg-ink/[0.05] hover:text-ink focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
      >
        {copied ? (
          <Check className="h-4 w-4 text-success" aria-hidden="true" />
        ) : (
          <Copy className="h-4 w-4" aria-hidden="true" />
        )}
      </button>
    </li>
  );
};

/**
 * First-run state: a box to dictate into, so the very first thing a new user
 * can do is the thing the app is for.
 */
const TryItBox: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const binding = getSetting("bindings")?.transcribe?.current_binding;
  const holdToTalk = getSetting("push_to_talk") ?? true;
  const fieldId = "home-try-dictation";

  return (
    <div className="rounded-2xl border border-hairline bg-surface p-4 elev-card">
      <label
        htmlFor={fieldId}
        className="flex flex-wrap items-center gap-x-2 gap-y-1.5 text-sm text-body"
      >
        <span>{t("home.recent.tryPrefix")}</span>
        <Keycaps binding={binding} size="sm" />
        <span>
          {holdToTalk
            ? t("home.recent.trySuffixHold")
            : t("home.recent.trySuffixTap")}
        </span>
      </label>
      <textarea
        id={fieldId}
        rows={3}
        placeholder={t("home.recent.tryPlaceholder")}
        className="mt-3 w-full resize-none rounded-xl border border-hairline bg-surface-muted px-3.5 py-3 text-sm text-ink placeholder:text-muted focus:border-accent focus:outline-none focus:ring-2 focus:ring-accent/20"
      />
    </div>
  );
};

export const RecentCard: React.FC = () => {
  const { t } = useTranslation();
  const { navigate } = useNavigation();
  const [entries, setEntries] = useState<HistoryEntry[] | null>(null);

  const load = useCallback(() => {
    void commands
      .getHistoryEntries(null, RECENT_LIMIT + 4)
      .then((result) => {
        if (result.status !== "ok") return;
        setEntries(
          result.data.entries
            .filter((entry) => finalTextOf(entry).length > 0)
            .slice(0, RECENT_LIMIT),
        );
      })
      .catch(() => setEntries([]));
  }, []);

  useEffect(() => {
    load();
    const unlisten = events.historyUpdatePayload.listen(() => load());
    return () => {
      void unlisten.then((off) => off());
    };
  }, [load]);

  const hasEntries = !!entries && entries.length > 0;

  return (
    <section>
      <SectionTitle
        title={t("home.recent.title")}
        action={
          hasEntries ? (
            <TextLink onClick={() => navigate("history")}>
              {t("home.recent.viewAll")}
            </TextLink>
          ) : undefined
        }
      />
      {entries === null ? (
        <div className="h-40 animate-pulse rounded-2xl border border-hairline bg-surface-muted" />
      ) : hasEntries ? (
        <ul className="divide-y divide-hairline overflow-hidden rounded-2xl border border-hairline bg-surface elev-card">
          {entries.map((entry) => (
            <RecentRow key={entry.id} entry={entry} />
          ))}
        </ul>
      ) : (
        <TryItBox />
      )}
    </section>
  );
};
