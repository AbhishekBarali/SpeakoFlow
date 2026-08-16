import React, { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  AlertTriangle,
  Bot,
  Check,
  FolderOpen,
  Square,
  Terminal as TerminalIcon,
  Trash2,
  X,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands, type AgentSessionView, type AgentStatus } from "@/bindings";
import { Button } from "../../ui/Button";
import { SectionHeader } from "../../ui/SectionHeader";
import { SettingsGroup } from "../../ui/SettingsGroup";

/** Payload of the backend's `agent-notification` event. */
interface AgentNotification {
  sessionId: string;
  label: string;
  message: string;
  highRisk: boolean;
}

/** Tone per status, so the list reads at a glance. Palette classes with dark
 *  variants, matching the convention in `ui/tones.ts` — there is no semantic
 *  `warning` token in the theme. */
const STATUS_TONE: Record<AgentStatus, string> = {
  starting: "bg-ink/8 text-body",
  working: "bg-accent/12 text-accent",
  waitingApproval:
    "bg-amber-500/15 text-amber-700 dark:bg-amber-400/20 dark:text-amber-300",
  idle: "bg-emerald-500/15 text-emerald-700 dark:bg-emerald-400/20 dark:text-emerald-300",
  failed: "bg-error/12 text-error",
  cancelled: "bg-ink/8 text-body",
  handedOff: "bg-sky-500/15 text-sky-700 dark:bg-sky-400/20 dark:text-sky-300",
  ended: "bg-ink/8 text-muted",
};

const formatElapsed = (secs: number): string => {
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m ${secs % 60}s`;
  return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m`;
};

export const AgentsSettings: React.FC = () => {
  const { t } = useTranslation();
  const [sessions, setSessions] = useState<AgentSessionView[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  // Which high-risk approval is awaiting its second, deliberate click.
  const [confirming, setConfirming] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const result = await commands.agentSessions();
    if (result.status === "ok") {
      setSessions(result.data);
    }
  }, []);

  useEffect(() => {
    void refresh();
    // Every transition pushes an event, so the list is live without polling.
    // The interval only exists to keep the elapsed times honest.
    const timer = window.setInterval(() => void refresh(), 5000);
    const unlistenUpdate = listen("agent-session-update", () => void refresh());
    const unlistenNotice = listen<AgentNotification>(
      "agent-notification",
      (event) => {
        const { message, highRisk } = event.payload;
        if (highRisk) {
          toast.warning(message);
        } else {
          toast(message);
        }
        void refresh();
      },
    );
    return () => {
      window.clearInterval(timer);
      void unlistenUpdate.then((fn) => fn());
      void unlistenNotice.then((fn) => fn());
    };
  }, [refresh]);

  /** Run one command, surface whatever it says, and resync. */
  const run = useCallback(
    async (key: string, action: () => Promise<{ status: string }>) => {
      setBusy(key);
      try {
        const result = (await action()) as
          | { status: "ok"; data: string }
          | { status: "error"; error: string };
        if (result.status === "ok") {
          toast.success(result.data);
        } else {
          toast.error(result.error);
        }
      } finally {
        setBusy(null);
        setConfirming(null);
        void refresh();
      }
    },
    [refresh],
  );

  const answer = useCallback(
    (id: string, allow: boolean, force: boolean) =>
      run(`${id}:permission`, () =>
        commands.agentSessionAnswerPermission(id, allow, force),
      ),
    [run],
  );

  const blocked = useMemo(
    () => sessions.filter((session) => session.pending !== null),
    [sessions],
  );

  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <SectionHeader
        title={t("sidebar.agents")}
        description={t("sectionSubtitles.agents")}
      />

      {/* Approvals come first: a blocked session is the only thing here that
          is actively costing the user time. */}
      {blocked.length > 0 && (
        <SettingsGroup
          title={t("settings.agents.approvals.title")}
          description={t("settings.agents.approvals.description")}
          icon={AlertTriangle}
        >
          {blocked.map((session) => {
            const pending = session.pending!;
            const key = `${session.id}:permission`;
            const awaitingConfirm = confirming === session.id;
            return (
              <div key={session.id} className="px-4 py-3 space-y-2">
                <div className="flex items-center gap-2">
                  <span className="text-[13px] font-medium text-ink">
                    {session.label}
                  </span>
                  <span className="text-xs text-muted">{pending.toolName}</span>
                  {pending.highRisk && (
                    <span className="text-[11px] px-1.5 py-0.5 rounded bg-error/12 text-error font-medium">
                      {t("settings.agents.approvals.highRisk")}
                    </span>
                  )}
                </div>
                <p className="text-xs text-body break-all font-mono">
                  {pending.detail}
                </p>
                {pending.highRisk && awaitingConfirm && (
                  <p className="text-xs text-error">
                    {t("settings.agents.approvals.confirmWarning")}
                  </p>
                )}
                <div className="flex gap-2 pt-0.5">
                  {pending.highRisk && !awaitingConfirm ? (
                    <Button
                      size="sm"
                      variant="danger"
                      disabled={busy === key}
                      onClick={() => setConfirming(session.id)}
                    >
                      <Check size={13} />
                      {t("settings.agents.approvals.approve")}
                    </Button>
                  ) : (
                    <Button
                      size="sm"
                      variant={pending.highRisk ? "danger" : "primary"}
                      disabled={busy === key}
                      onClick={() =>
                        void answer(session.id, true, pending.highRisk)
                      }
                    >
                      <Check size={13} />
                      {pending.highRisk
                        ? t("settings.agents.approvals.confirmApprove")
                        : t("settings.agents.approvals.approve")}
                    </Button>
                  )}
                  <Button
                    size="sm"
                    variant="secondary"
                    disabled={busy === key}
                    onClick={() => void answer(session.id, false, false)}
                  >
                    <X size={13} />
                    {t("settings.agents.approvals.deny")}
                  </Button>
                </div>
              </div>
            );
          })}
        </SettingsGroup>
      )}

      <SettingsGroup
        title={t("settings.agents.sessions.title")}
        description={t("settings.agents.sessions.description")}
        icon={Bot}
      >
        {sessions.length === 0 ? (
          <div className="px-4 py-6 text-center space-y-1">
            <p className="text-[13px] text-body">
              {t("settings.agents.empty.title")}
            </p>
            <p className="text-xs text-muted">
              {t("settings.agents.empty.hint")}
            </p>
          </div>
        ) : (
          sessions.map((session) => {
            // Stop only means something while a turn is in flight; offering it
            // on a finished session made the button look broken.
            const running =
              session.status === "starting" ||
              session.status === "working" ||
              session.status === "waitingApproval";
            return (
              <div key={session.id} className="px-4 py-3 space-y-2">
                <div className="flex items-center gap-2 flex-wrap">
                  <span className="text-[13px] font-medium text-ink">
                    {session.label}
                  </span>
                  <span
                    className={`text-[11px] px-1.5 py-0.5 rounded font-medium ${STATUS_TONE[session.status]}`}
                  >
                    {t(`settings.agents.status.${session.status}`)}
                  </span>
                  <span className="text-xs text-muted">
                    {formatElapsed(session.elapsedSecs)}
                  </span>
                  {session.lastTool && (
                    <span className="text-xs text-muted">
                      {session.lastTool}
                    </span>
                  )}
                  {session.filesTouched.length > 0 && (
                    <span className="text-xs text-muted">
                      {t("settings.agents.filesChanged", {
                        count: session.filesTouched.length,
                      })}
                    </span>
                  )}
                  {session.costUsd > 0 && (
                    <span className="text-xs text-muted">
                      {t("settings.agents.cost", {
                        amount: session.costUsd.toFixed(2),
                      })}
                    </span>
                  )}
                </div>

                <p className="text-xs text-muted break-all">{session.cwd}</p>
                <p className="text-xs text-body">{session.task}</p>
                {session.lastLine && (
                  <p className="text-xs text-body italic">{session.lastLine}</p>
                )}
                {/* A tool that failed mid-turn: the session is still running, so
                    this is a warning rather than an error. */}
                {session.toolError && (
                  <p className="text-xs text-amber-700 dark:text-amber-300">
                    {t("settings.agents.toolFailed", {
                      detail: session.toolError,
                    })}
                  </p>
                )}
                {session.error && (
                  <p className="text-xs text-error">{session.error}</p>
                )}

                <div className="flex gap-2 pt-0.5 flex-wrap">
                  <Button
                    size="sm"
                    variant="secondary"
                    disabled={!running || busy === `${session.id}:cancel`}
                    onClick={() =>
                      void run(`${session.id}:cancel`, () =>
                        commands.agentSessionCancel(session.id),
                      )
                    }
                  >
                    <Square size={13} />
                    {t("settings.agents.actions.stop")}
                  </Button>
                  <Button
                    size="sm"
                    variant="primary-soft"
                    // Deliberately still enabled once handed off: opening another
                    // terminal is harmless, and disabling it turned a failed
                    // handoff into a dead end with no way back.
                    disabled={busy === `${session.id}:terminal`}
                    title={t("settings.agents.actions.terminalHint")}
                    onClick={() =>
                      void run(`${session.id}:terminal`, () =>
                        commands.agentSessionResumeInTerminal(session.id),
                      )
                    }
                  >
                    <TerminalIcon size={13} />
                    {t("settings.agents.actions.terminal")}
                  </Button>
                  <Button
                    size="sm"
                    variant="secondary"
                    disabled={busy === `${session.id}:folder`}
                    onClick={() =>
                      void run(`${session.id}:folder`, () =>
                        commands.agentSessionOpenFolder(session.id),
                      )
                    }
                  >
                    <FolderOpen size={13} />
                    {t("settings.agents.actions.openFolder")}
                  </Button>
                  <Button
                    size="sm"
                    variant="danger-ghost"
                    disabled={busy === `${session.id}:close`}
                    title={t("settings.agents.actions.closeHint")}
                    onClick={() =>
                      void run(`${session.id}:close`, () =>
                        commands.agentSessionClose(session.id),
                      )
                    }
                  >
                    <Trash2 size={13} />
                    {t("settings.agents.actions.close")}
                  </Button>
                </div>
              </div>
            );
          })
        )}
      </SettingsGroup>
    </div>
  );
};
