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
    <div className="w-full rounded-2xl border border-error/40 bg-error/10 p-4">
      <div className="flex items-start gap-3">
        <span className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-xl bg-error/15 text-error">
          <MicOff size={17} />
        </span>
        <div className="min-w-0 flex-1">
          <p className="text-[13.5px] font-semibold text-red-700 dark:text-red-300">
            {live
              ? t("meetings.systemAudio.liveTitle")
              : t("meetings.systemAudio.unavailableTitle")}
          </p>
          <p className="mt-1 text-[13px] leading-relaxed text-red-700/90 dark:text-red-300/90">
            {live
              ? t("meetings.systemAudio.liveBody")
              : t("meetings.systemAudio.unavailableBody")}
          </p>
          {detail && (
            <p className="mt-2 rounded-lg bg-error/10 px-2.5 py-1.5 font-mono text-[11.5px] leading-relaxed break-words text-red-700 dark:text-red-300">
              {detail}
            </p>
          )}
        </div>
      </div>
    </div>
  );
};
