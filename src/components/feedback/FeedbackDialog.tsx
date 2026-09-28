import React, { useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import {
  Bug,
  CircleCheck,
  Lightbulb,
  LoaderCircle,
  MessageCircleQuestionMark,
} from "lucide-react";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { Textarea } from "../ui/Textarea";
import { Input } from "../ui/Input";
import { Segmented } from "../ui/Segmented";
import {
  MAX_MESSAGE_CHARS,
  canSend,
  clearDraft,
  describeSystem,
  getFeedbackSystemInfo,
  loadDraft,
  loadEmail,
  looksLikeEmail,
  messageLength,
  saveDraft,
  saveEmail,
  sendFeedback,
  useFeedbackDialog,
  type FeedbackKind,
  type FeedbackSystemInfo,
} from "./feedbackStore";

/**
 * "Tell the developer": one short form — what kind, what happened, and an
 * email only if you want a reply. Opened from the "?" in the sidebar, the tray
 * menu, or Settings → About; it never opens on its own.
 *
 * Everything that will be sent is on screen before Send: the message, the
 * email if one was typed, and the exact system line when its box is ticked.
 */
export const FeedbackDialog: React.FC = () => {
  const { t } = useTranslation();
  const { open, kind: requestedKind, hide } = useFeedbackDialog();
  const ids = useId();

  const [kind, setKind] = useState<FeedbackKind>("bug");
  const [message, setMessage] = useState("");
  const [email, setEmail] = useState("");
  const [includeSystem, setIncludeSystem] = useState(true);
  const [system, setSystem] = useState<FeedbackSystemInfo | null>(null);
  const [status, setStatus] = useState<"editing" | "sending" | "sent">(
    "editing",
  );
  const [error, setError] = useState<string | null>(null);
  const [sentTo, setSentTo] = useState<string | null>(null);

  // Restore the draft each time the dialog opens.
  useEffect(() => {
    if (!open) return;
    const draft = loadDraft();
    setKind(requestedKind ?? draft.kind);
    setMessage(draft.message);
    setEmail(loadEmail());
    setStatus("editing");
    setError(null);
    getFeedbackSystemInfo()
      .then(setSystem)
      .catch(() => setSystem(null));
  }, [open, requestedKind]);

  useEffect(() => {
    if (open && status === "editing") saveDraft({ kind, message });
  }, [open, status, kind, message]);

  const trimmedEmail = email.trim();
  const emailInvalid = trimmedEmail !== "" && !looksLikeEmail(trimmedEmail);
  const length = messageLength(message);
  const ready = canSend(message, email) && status === "editing";

  const submit = async () => {
    if (!ready) return;
    setStatus("sending");
    setError(null);
    try {
      await sendFeedback({
        kind,
        message,
        email: trimmedEmail || null,
        include_system_info: includeSystem,
      });
      saveEmail(trimmedEmail);
      clearDraft();
      setSentTo(trimmedEmail || null);
      setMessage("");
      setStatus("sent");
    } catch (err) {
      setError(typeof err === "string" ? err : t("feedback.errors.generic"));
      setStatus("editing");
    }
  };

  const kinds: { value: FeedbackKind; icon: typeof Bug }[] = [
    { value: "bug", icon: Bug },
    { value: "idea", icon: Lightbulb },
    { value: "question", icon: MessageCircleQuestionMark },
  ];

  const sent = status === "sent";

  const footer = sent ? (
    <div className="ms-auto flex gap-2">
      <Button variant="ghost" size="sm" onClick={() => setStatus("editing")}>
        {t("feedback.sendAnother")}
      </Button>
      <Button variant="primary" size="sm" onClick={hide}>
        {t("common.close")}
      </Button>
    </div>
  ) : (
    <>
      <p className="me-auto text-xs text-muted">{t("feedback.privacy")}</p>
      <Button variant="ghost" size="sm" onClick={hide}>
        {t("common.cancel")}
      </Button>
      <Button
        variant="primary"
        size="sm"
        disabled={!ready}
        onClick={() => void submit()}
      >
        {status === "sending" && (
          <LoaderCircle
            className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none"
            aria-hidden="true"
          />
        )}
        {status === "sending" ? t("feedback.sending") : t("feedback.send")}
      </Button>
    </>
  );

  return (
    <Dialog
      open={open}
      onClose={hide}
      size="md"
      title={t("feedback.title")}
      description={sent ? undefined : t("feedback.description")}
      footer={footer}
      // The draft is saved as it is typed, so a stray click outside loses
      // nothing; only an in-flight send holds the dialog open.
      closeOnScrim={status !== "sending"}
    >
      {sent ? (
        <div
          role="status"
          className="flex flex-col items-center px-2 pt-8 pb-4 text-center"
        >
          <CircleCheck
            className="h-10 w-10 text-success"
            strokeWidth={1.6}
            aria-hidden="true"
          />
          <p className="mt-3 font-display text-[1.25rem] text-ink">
            {t("feedback.sentTitle")}
          </p>
          <p className="mt-1.5 max-w-sm text-sm text-muted">
            {sentTo
              ? t("feedback.sentWithEmail", { email: sentTo })
              : t("feedback.sentBody")}
          </p>
        </div>
      ) : (
        <form
          className="space-y-4"
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <Segmented
            label={t("feedback.kind.label")}
            value={kind}
            onChange={setKind}
            fill
            options={kinds.map(({ value, icon }) => ({
              value,
              icon,
              label: t(`feedback.kind.${value}`),
            }))}
          />

          <div>
            <label htmlFor={`${ids}-message`} className="sr-only">
              {t("feedback.messageLabel")}
            </label>
            <Textarea
              id={`${ids}-message`}
              data-autofocus=""
              value={message}
              maxLength={MAX_MESSAGE_CHARS * 2}
              onChange={(event) => setMessage(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
                  event.preventDefault();
                  void submit();
                }
              }}
              placeholder={t(`feedback.placeholder.${kind}`)}
              rows={6}
              className="w-full resize-none"
            />
            {length > MAX_MESSAGE_CHARS - 500 && (
              <p
                className={`mt-1 text-end text-xs ${length > MAX_MESSAGE_CHARS ? "text-error" : "text-muted"}`}
              >
                {t("feedback.remaining", {
                  count: MAX_MESSAGE_CHARS - length,
                })}
              </p>
            )}
          </div>

          <div>
            <label
              htmlFor={`${ids}-email`}
              className="mb-1.5 flex items-baseline gap-1.5 text-sm font-medium text-ink"
            >
              {t("feedback.emailLabel")}
              <span className="text-xs font-normal text-muted">
                {t("feedback.emailOptional")}
              </span>
            </label>
            <Input
              id={`${ids}-email`}
              type="email"
              inputMode="email"
              autoComplete="email"
              value={email}
              onChange={(event) => setEmail(event.target.value)}
              placeholder={t("feedback.emailPlaceholder")}
              aria-invalid={emailInvalid || undefined}
              aria-describedby={`${ids}-email-hint`}
              className="w-full"
            />
            <p
              id={`${ids}-email-hint`}
              className={`mt-1 text-xs ${emailInvalid ? "text-error" : "text-muted"}`}
            >
              {emailInvalid
                ? t("feedback.emailInvalid")
                : t("feedback.emailHint")}
            </p>
          </div>

          <label className="flex cursor-pointer items-start gap-2.5">
            <input
              type="checkbox"
              checked={includeSystem}
              onChange={(event) => setIncludeSystem(event.target.checked)}
              className="mt-0.5 h-4 w-4 shrink-0 cursor-pointer accent-accent"
            />
            <span className="min-w-0">
              <span className="block text-sm text-ink">
                {t("feedback.includeSystem")}
              </span>
              {system && (
                <span className="block truncate text-xs text-muted">
                  {describeSystem(system)}
                </span>
              )}
            </span>
          </label>

          {error && (
            <p role="alert" className="text-sm text-error">
              {error}
            </p>
          )}
        </form>
      )}
    </Dialog>
  );
};

/** Mounts the dialog once and opens it when the tray asks. */
export const FeedbackHost: React.FC = () => {
  const show = useFeedbackDialog((s) => s.show);
  useEffect(() => {
    const unlisten = listen("open-feedback", () => show());
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [show]);
  return <FeedbackDialog />;
};
