import React from "react";
import { useTranslation } from "react-i18next";
import { MicOff } from "lucide-react";

interface SystemAudioNoticeProps {
  /** True once a recording is running without the system stream. */
  live: boolean;
  /** `MeetingState.system_audio_error`, or the platform help text before a
   *  recording has started. */
  detail: string | null;
}

/**
 * The loudest thing on the page when the other participants cannot be heard.
 *
 * Deliberately not a quiet caption or an (i) hint. A meeting that captures only
 * the user's own voice is nearly useless — half a conversation, with the half
 * that contains the decisions missing — and someone who does not notice until
 * afterwards has lost the meeting. It is also not dismissible: hiding the reason
 * something is broken is worse than the interruption of saying it.
 *
 * The microphone side keeps recording either way, which is why this is a warning
 * rather than a refusal: half a meeting is still better than none, as long as the
 * user knows that is what they are getting.
 */
export const SystemAudioNotice: React.FC<SystemAudioNoticeProps> = ({
  live,
  detail,
}) => {
  const { t } = useTranslation();

  return (
    <div
      className="w-full rounded-2xl border border-error/30 bg-error/[0.06] px-5 py-4"
      role="alert"
    >
      <div className="flex items-start gap-3.5">
        <span className="grid h-9 w-9 shrink-0 place-items-center rounded-xl bg-error/12 text-error">
          <MicOff className="h-[18px] w-[18px]" aria-hidden="true" />
        </span>
        <div className="min-w-0 flex-1">
          <p className="text-[0.9375rem] font-semibold text-ink">
            {live
              ? t("meetings.systemAudio.liveTitle")
              : t("meetings.systemAudio.unavailableTitle")}
          </p>
          <p className="mt-1 max-w-[68ch] text-sm leading-relaxed text-body">
            {live
              ? t("meetings.systemAudio.liveBody")
              : t("meetings.systemAudio.unavailableBody")}
          </p>
          {detail && (
            <p className="mt-2.5 rounded-lg border border-hairline bg-surface px-3 py-2 font-mono text-xs leading-relaxed break-words text-muted">
              {detail}
            </p>
          )}
        </div>
      </div>
    </div>
  );
};
