import React, { useCallback, useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import {
  Bug,
  CircleCheck,
  ImagePlus,
  Lightbulb,
  LoaderCircle,
  MessageCircleQuestionMark,
  X,
} from "lucide-react";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { InfoTip } from "../ui/InfoTip";
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
import {
  MAX_SCREENSHOTS,
  ScreenshotRejected,
  compressScreenshot,
  formatBytes,
  imagesFromClipboard,
  type Screenshot,
} from "./screenshots";

/**
 * "Tell the developer": what kind, what happened, a screenshot if it helps,
 * and an email only if you want a reply. Opened from the "?" in the sidebar,
 * the tray menu, or Settings → About; it never opens on its own.
 *
 * The form is one composer rather than a stack of labelled fields: the
 * message and its screenshots share a box, the email is a single line whose
 * placeholder says it is optional, and the system line sits in the footer
 * with its exact text behind an (i). The earlier layout labelled, hinted and
 * explained every field, and the explanations were what made it hard to read.
 *
 * Screenshots are pasted (anywhere in the dialog) or picked, and each is
 * shrunk in `screenshots.ts` before it is shown, so the thumbnail is exactly
 * what will be sent. They live only in memory: the text draft survives a
 * restart, but a few megabytes of images do not belong in localStorage.
 */

type Attachment =
  | { id: number; status: "working" }
  | { id: number; status: "ready"; shot: Screenshot };

let nextAttachmentId = 1;

const pasteShortcut = () =>
  typeof navigator !== "undefined" && /Mac/i.test(navigator.platform)
    ? "⌘V"
    : "Ctrl+V";

export const FeedbackDialog: React.FC = () => {
  const { t } = useTranslation();
  const { open, kind: requestedKind, hide } = useFeedbackDialog();
  const ids = useId();
  const fileInput = useRef<HTMLInputElement>(null);

  const [kind, setKind] = useState<FeedbackKind>("bug");
  const [message, setMessage] = useState("");
  const [email, setEmail] = useState("");
  const [includeSystem, setIncludeSystem] = useState(true);
  const [system, setSystem] = useState<FeedbackSystemInfo | null>(null);
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [attachError, setAttachError] = useState<string | null>(null);
  const [status, setStatus] = useState<"editing" | "sending" | "sent">(
    "editing",
  );
  const [error, setError] = useState<string | null>(null);
  const [sentTo, setSentTo] = useState<string | null>(null);
  const [dropped, setDropped] = useState(0);

  // Restore the draft each time the dialog opens.
  useEffect(() => {
    if (!open) return;
    const draft = loadDraft();
    setKind(requestedKind ?? draft.kind);
    setMessage(draft.message);
    setEmail(loadEmail());
    setStatus("editing");
    setError(null);
    setAttachError(null);
    getFeedbackSystemInfo()
      .then(setSystem)
      .catch(() => setSystem(null));
  }, [open, requestedKind]);

  useEffect(() => {
    if (open && status === "editing") saveDraft({ kind, message });
  }, [open, status, kind, message]);

  const attachmentsRef = useRef(attachments);
  attachmentsRef.current = attachments;

  const addFiles = useCallback(
    (files: File[]) => {
      if (files.length === 0) return;
      setAttachError(null);
      const room = MAX_SCREENSHOTS - attachmentsRef.current.length;
      if (files.length > room) {
        setAttachError(
          t("feedback.screenshots.limit", { count: MAX_SCREENSHOTS }),
        );
      }
      const accepted = files.slice(0, Math.max(0, room));
      if (accepted.length === 0) return;
      const pending: Attachment[] = accepted.map(() => ({
        id: nextAttachmentId++,
        status: "working",
      }));
      // Counted now, not when React commits, so two quick pastes cannot both
      // see room for three.
      attachmentsRef.current = [...attachmentsRef.current, ...pending];
      setAttachments((current) => [...current, ...pending]);

      accepted.forEach((file, index) => {
        const { id } = pending[index];
        compressScreenshot(file)
          .then((shot) =>
            setAttachments((list) =>
              list.map((item) =>
                item.id === id ? { id, status: "ready", shot } : item,
              ),
            ),
          )
          .catch((reason: unknown) => {
            setAttachments((list) => list.filter((item) => item.id !== id));
            const key =
              reason instanceof ScreenshotRejected
                ? reason.reason
                : "unreadable";
            setAttachError(t(`feedback.screenshots.${key}`));
          });
      });
    },
    [t],
  );

  // Paste works wherever focus is in the dialog, not only in the message box:
  // after clicking a thumbnail or the kind picker, Ctrl+V should still attach.
  useEffect(() => {
    if (!open || status !== "editing") return;
    const onPaste = (event: ClipboardEvent) => {
      const files = imagesFromClipboard(event.clipboardData);
      if (files.length === 0) return;
      event.preventDefault();
      addFiles(files);
    };
    document.addEventListener("paste", onPaste);
    return () => document.removeEventListener("paste", onPaste);
  }, [open, status, addFiles]);

  const trimmedEmail = email.trim();
  const emailInvalid = trimmedEmail !== "" && !looksLikeEmail(trimmedEmail);
  const length = messageLength(message);
  const working = attachments.some((item) => item.status === "working");
  const ready = canSend(message, email) && status === "editing" && !working;

  const submit = async () => {
    if (!ready) return;
    setStatus("sending");
    setError(null);
    try {
      const outcome = await sendFeedback({
        kind,
        message,
        email: trimmedEmail || null,
        include_system_info: includeSystem,
        attachments: attachments.flatMap((item) =>
          item.status === "ready"
            ? [{ media_type: item.shot.mediaType, data: item.shot.data }]
            : [],
        ),
      });
      saveEmail(trimmedEmail);
      clearDraft();
      setSentTo(trimmedEmail || null);
      setDropped(outcome?.attachments_dropped ?? 0);
      setMessage("");
      setAttachments([]);
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
  const full = attachments.length >= MAX_SCREENSHOTS;
  const nearLimit = length > MAX_MESSAGE_CHARS - 500;

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
      <label className="me-auto flex min-w-0 cursor-pointer items-center gap-2 text-[0.8125rem] text-muted">
        <input
          type="checkbox"
          checked={includeSystem}
          onChange={(event) => setIncludeSystem(event.target.checked)}
          className="h-3.5 w-3.5 shrink-0 cursor-pointer accent-accent"
        />
        <span className="truncate">{t("feedback.includeSystem")}</span>
        {system && (
          <InfoTip
            text={t("feedback.systemDetails", {
              system: describeSystem(system),
            })}
          />
        )}
      </label>
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
          className="flex flex-col items-center px-2 pt-6 pb-2 text-center"
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
          {dropped > 0 && (
            <p className="mt-2 max-w-sm text-xs text-muted">
              {t("feedback.screenshotsDropped", { count: dropped })}
            </p>
          )}
        </div>
      ) : (
        <form
          className="space-y-3"
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <Segmented
            label={t("feedback.kind.label")}
            value={kind}
            onChange={setKind}
            size="sm"
            options={kinds.map(({ value, icon }) => ({
              value,
              icon,
              label: t(`feedback.kind.${value}`),
            }))}
          />

          {/* The composer: message, screenshots, and the attach control in one box. */}
          <div className="rounded-xl border border-hairline-strong bg-surface transition-colors duration-150 focus-within:border-ink hover:border-ink/40 focus-within:hover:border-ink">
            <label htmlFor={`${ids}-message`} className="sr-only">
              {t("feedback.messageLabel")}
            </label>
            <textarea
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
              rows={5}
              className="block w-full resize-none rounded-t-xl bg-transparent px-3.5 pt-3 pb-1 text-sm leading-relaxed text-ink placeholder:text-muted-soft focus:outline-none"
            />

            {attachments.length > 0 && (
              <ul className="flex flex-wrap gap-2 px-3 pt-1 pb-1">
                {attachments.map((item, index) => (
                  <li
                    key={item.id}
                    className="relative h-16 w-24 overflow-hidden rounded-lg border border-hairline bg-canvas"
                  >
                    {item.status === "working" ? (
                      <span
                        role="status"
                        aria-label={t("feedback.screenshots.working")}
                        className="grid h-full w-full place-items-center text-muted"
                      >
                        <LoaderCircle
                          className="h-4 w-4 animate-spin motion-reduce:animate-none"
                          aria-hidden="true"
                        />
                      </span>
                    ) : (
                      <>
                        <img
                          src={`data:${item.shot.mediaType};base64,${item.shot.data}`}
                          alt={t("feedback.screenshots.preview", {
                            index: index + 1,
                            width: item.shot.width,
                            height: item.shot.height,
                            size: formatBytes(item.shot.bytes),
                          })}
                          title={t("feedback.screenshots.preview", {
                            index: index + 1,
                            width: item.shot.width,
                            height: item.shot.height,
                            size: formatBytes(item.shot.bytes),
                          })}
                          className="h-full w-full object-cover object-top"
                        />
                        <button
                          type="button"
                          onClick={() => {
                            setAttachError(null);
                            setAttachments((list) =>
                              list.filter((other) => other.id !== item.id),
                            );
                          }}
                          aria-label={t("feedback.screenshots.remove", {
                            index: index + 1,
                          })}
                          title={t("feedback.screenshots.remove", {
                            index: index + 1,
                          })}
                          className="absolute end-1 top-1 grid h-5 w-5 cursor-pointer place-items-center rounded-full bg-[rgb(10_20_24/0.72)] text-white transition-colors hover:bg-[rgb(10_20_24/0.9)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/60"
                        >
                          <X className="h-3 w-3" aria-hidden="true" />
                        </button>
                      </>
                    )}
                  </li>
                ))}
              </ul>
            )}

            <div className="flex items-center gap-2 px-2 pt-1 pb-2">
              <button
                type="button"
                onClick={() => fileInput.current?.click()}
                disabled={full}
                className="inline-flex h-7 cursor-pointer items-center gap-1.5 rounded-md px-2 text-xs font-medium text-muted transition-colors hover:bg-ink/[0.05] hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent"
              >
                <ImagePlus className="h-3.5 w-3.5" aria-hidden="true" />
                {t("feedback.screenshots.add")}
              </button>
              {!full && (
                <span className="truncate text-xs text-muted-soft">
                  {t("feedback.screenshots.pasteHint", {
                    shortcut: pasteShortcut(),
                  })}
                </span>
              )}
              {nearLimit && (
                <span
                  className={`ms-auto shrink-0 pe-1.5 text-xs tabular-nums ${length > MAX_MESSAGE_CHARS ? "text-error" : "text-muted"}`}
                >
                  {t("feedback.remaining", {
                    count: MAX_MESSAGE_CHARS - length,
                  })}
                </span>
              )}
              <input
                ref={fileInput}
                type="file"
                accept="image/png,image/jpeg,image/webp,image/gif,image/bmp"
                multiple
                hidden
                onChange={(event) => {
                  addFiles(Array.from(event.target.files ?? []));
                  // Picking the same file again must still fire a change.
                  event.target.value = "";
                }}
              />
            </div>
          </div>

          {attachError && (
            <p role="alert" className="text-xs text-error">
              {attachError}
            </p>
          )}

          <div>
            <label htmlFor={`${ids}-email`} className="sr-only">
              {t("feedback.emailLabel")}
            </label>
            <input
              id={`${ids}-email`}
              type="email"
              inputMode="email"
              autoComplete="email"
              value={email}
              onChange={(event) => setEmail(event.target.value)}
              placeholder={t("feedback.emailPlaceholder")}
              aria-invalid={emailInvalid || undefined}
              aria-describedby={emailInvalid ? `${ids}-email-error` : undefined}
              className={`w-full rounded-lg border bg-surface px-3.5 py-2 text-sm text-ink transition-colors duration-150 placeholder:text-muted-soft hover:border-ink/40 focus:border-ink focus:outline-none ${emailInvalid ? "border-error" : "border-hairline-strong"}`}
            />
            {emailInvalid && (
              <p id={`${ids}-email-error`} className="mt-1 text-xs text-error">
                {t("feedback.emailInvalid")}
              </p>
            )}
          </div>

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
