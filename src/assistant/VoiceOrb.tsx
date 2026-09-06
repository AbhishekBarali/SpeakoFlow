import type { CSSProperties } from "react";
import type { ConversationPhase } from "./useVoiceConversation";

/** One presence across the focused view and the compact, always-listening pill. */
export function VoiceOrb({
  phase,
  level = 0,
}: {
  phase: ConversationPhase;
  level?: number;
}) {
  return (
    <div
      className={`voice-orb ${phase}`}
      style={
        { "--voice-energy": phase === "hearing" ? level : 0 } as CSSProperties
      }
      aria-hidden="true"
    >
      <div className="voice-orb-body">
        <span className="voice-orb-flow" />
        <span className="voice-orb-light" />
      </div>
    </div>
  );
}
