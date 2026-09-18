import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { MicVAD } from "@ricky0123/vad-web";
import { ConversationAudio } from "./conversationAudio";
import {
  DEFAULT_CONVERSATION_PACE,
  MAX_UTTERANCE_MS,
  matchDeviceByName,
  sameVoiceTicket,
  TURN_PAUSE_MS,
  VoiceTurnGate,
  type ConversationPace,
  type VoiceTicket,
} from "./conversationPolicy";

export type ConversationPhase =
  | "off"
  | "starting"
  | "listening"
  | "hearing"
  | "transcribing"
  | "responding"
  | "speaking"
  | "muted"
  | "deafened"
  | "error";
type VoiceError = {
  code: "microphone" | "device" | "setup" | "turn" | "playback" | "tooLong";
  detail?: string;
};
interface VoiceCallbacks {
  stopLocal: () => void;
  beginLocal: (epoch: number) => Promise<void>;
  pushLocal: (text: string) => void;
  endLocal: () => void;
  microphone?: string | null;
  outputDevice?: string | null;
  volume?: number;
  /**
   * The persisted pace (`assistant_conversation_pace`). Owned by settings rather
   * than by this hook so it survives a restart and a panel-window reload — a
   * user who needs Patient needs it in every call.
   */
  pace?: ConversationPace | null;
  onPaceChange: (pace: ConversationPace) => void;
  /**
   * Hang up: end the session *and* send the surface away.
   *
   * The hook can only do the first half, and on its own that is the bug — the
   * window stays up and re-renders as the quick-ask card, so ending a call looks
   * like a second, different assistant opening itself. Whoever owns the window
   * supplies this; without it, Escape falls back to a local-only end.
   */
  onHangUp?: () => void;
}
interface Session {
  id: number;
  ticket: VoiceTicket | null;
  vad: MicVAD | null;
  stream: MediaStream | null;
  context: AudioContext;
  audio: ConversationAudio;
  /**
   * Microphone off. Input only: a muted call still speaks.
   *
   * This used to mean both directions at once, which is what made the button
   * confusing — muting to stop the assistant hearing a conversation in the room
   * also cut off the answer being read out, and there was no way to ask for one
   * without the other. `deafened` is now the switch for the output side.
   */
  muted: boolean;
  /** Both directions off: nothing is heard and nothing is spoken. */
  deafened: boolean;
  /** A VAD pause/start is in flight; the two switches share it. */
  changingAudio: boolean;
  hearing: boolean;
  turnDone: boolean;
  synthesisDone: boolean;
  playing: boolean;
  epoch: number | null;
  onset: Promise<VoiceTicket> | null;
  timeout: ReturnType<typeof setTimeout> | null;
  releaseBackgroundLock?: () => void;
}

/**
 * The microphone is live only when neither switch is down.
 *
 * Deafening closes it too, because "you won't hear it and it won't take your
 * input" is one gesture: leaving capture running behind a silenced call would
 * keep answering questions whose replies nobody can hear.
 */
const micLive = (s: Session) => !s.muted && !s.deafened;

/** Only deafening silences the assistant. Mute is the input side alone. */
const canHear = (s: Session) => !s.deafened;

export function useVoiceConversation(callbacks: VoiceCallbacks) {
  const callbacksRef = useRef(callbacks);
  callbacksRef.current = callbacks;
  const [open, setOpen] = useState(false);
  const openRef = useRef(open);
  openRef.current = open;
  const [phase, setPhase] = useState<ConversationPhase>("off");
  const [error, setError] = useState<VoiceError | null>(null);
  const [level, setLevel] = useState(0);
  /**
   * The two switches, exposed as state rather than read back off `phase`.
   *
   * `phase` says what the call is *doing*, and a muted call still does things —
   * it thinks and it speaks. Deriving the button's pressed state from a "muted"
   * phase forced the phase to mask "Thinking"/"Speaking" for the whole time the
   * microphone was off, which is the half of the old mute that read as a bug.
   */
  const [muted, setMuted] = useState(false);
  const [deafened, setDeafened] = useState(false);
  const pace = callbacks.pace ?? DEFAULT_CONVERSATION_PACE;
  const setPace = callbacks.onPaceChange;
  const paceRef = useRef(pace);
  paceRef.current = pace;
  const sessionRef = useRef<Session | null>(null);
  const lifecycle = useRef(new VoiceTurnGate());
  const turns = useRef(new VoiceTurnGate());
  const ready = useRef<Promise<unknown>>(Promise.resolve());
  const backendLifecycle = useRef<Promise<unknown>>(Promise.resolve());
  /**
   * Backend session ids this hook has already closed.
   *
   * `assistant-conversation-ended` has to be honoured by a session that does not
   * yet know its own id — see the listener — and that alone would let a *previous*
   * session's late event tear down a call that had just started. Remembering what
   * we have already released settles which of the two an event belongs to without
   * guessing from timing.
   */
  const closedSessions = useRef(new Set<number>());
  const lastLevelAt = useRef(0);

  const refreshPhase = useCallback((s: Session) => {
    if (sessionRef.current !== s) return;
    setPhase(
      s.deafened
        ? "deafened"
        : s.hearing
          ? "hearing"
          : s.playing
            ? "speaking"
            : !s.turnDone || !s.synthesisDone
              ? "responding"
              : s.muted
                ? "muted"
                : "listening",
    );
  }, []);

  const release = useCallback((s: Session) => {
    s.releaseBackgroundLock?.();
    if (s.timeout) clearTimeout(s.timeout);
    s.stream?.getTracks().forEach((track) => track.stop());
    s.audio.stop();
    void s.vad?.destroy().catch(() => {});
    void s.context.close().catch(() => {});
    if (s.id) {
      closedSessions.current.add(s.id);
      backendLifecycle.current = backendLifecycle.current
        .then(() => invoke("assistant_conversation_end", { session: s.id }))
        .catch(() => {});
    }
  }, []);

  const end = useCallback(() => {
    lifecycle.current.next();
    turns.current.next();
    const s = sessionRef.current;
    sessionRef.current = null;
    if (s) {
      callbacksRef.current.stopLocal();
      release(s);
    }
    setOpen(false);
    setPhase("off");
    setError(null);
    setLevel(0);
    setMuted(false);
    setDeafened(false);
  }, [release]);

  const fail = useCallback(
    (failure: VoiceError) => {
      end();
      setOpen(true);
      setPhase("error");
      setError(failure);
    },
    [end],
  );

  /**
   * One failed turn is not a failed session. A provider that rejects a request,
   * or an utterance that could not be submitted, says nothing about the
   * microphone — so show what went wrong and keep listening instead of tearing
   * the session down and dropping the user back into the chat view. The next
   * time they speak, `onSpeechRealStart` clears the message.
   */
  const recover = useCallback(
    (failure: VoiceError, session: Session) => {
      if (sessionRef.current !== session) return;
      session.turnDone = true;
      session.synthesisDone = true;
      setError(failure);
      refreshPhase(session);
    },
    [refreshPhase],
  );

  const interrupt = useCallback((s: Session) => {
    const interruptedReply = s.playing || !s.synthesisDone;
    turns.current.next();
    s.ticket = null;
    s.epoch = null;
    s.audio.stop();
    callbacksRef.current.stopLocal();
    s.turnDone = true;
    s.synthesisDone = true;
    s.onset = invoke<VoiceTicket>("assistant_conversation_interrupt", {
      session: s.id,
      interruptedReply,
    });
    // The rejection is handled on submission; no unhandled rejection if the
    // user mutes/ends while a speech-start command is crossing IPC.
    void s.onset.catch(() => {});
    return s.onset;
  }, []);

  const submit = useCallback(
    async (s: Session, audio: Float32Array) => {
      if (sessionRef.current !== s || !micLive(s)) return;
      if (s.timeout) clearTimeout(s.timeout);
      s.timeout = null;
      s.hearing = false;
      const generation = turns.current.next();
      setLevel(0);
      setPhase("transcribing");
      try {
        if (!s.onset) {
          refreshPhase(s);
          return;
        }
        const ticket = await s.onset;
        if (
          sessionRef.current !== s ||
          !turns.current.accepts(generation) ||
          !micLive(s)
        )
          return;
        s.ticket = ticket;
        s.turnDone = false;
        s.synthesisDone = true;
        await invoke(
          "assistant_conversation_audio",
          audio.buffer.slice(
            audio.byteOffset,
            audio.byteOffset + audio.byteLength,
          ),
          {
            headers: {
              "x-voice-session": String(ticket.session),
              "x-voice-turn": String(ticket.turn),
            },
          },
        );
        if (sessionRef.current !== s || !turns.current.accepts(generation))
          return;
        s.turnDone = true;
        s.onset = null;
        refreshPhase(s);
      } catch (cause) {
        if (sessionRef.current !== s || !turns.current.accepts(generation))
          return;
        recover({ code: "turn", detail: String(cause) }, s);
      }
    },
    [recover, refreshPhase],
  );

  const start = useCallback(async () => {
    end();
    callbacksRef.current.stopLocal();
    const generation = lifecycle.current.next();
    setOpen(true);
    setError(null);
    setPhase("starting");
    let s: Session | null = null;
    try {
      const context = new AudioContext();
      const session: Session = {
        id: 0,
        ticket: null,
        vad: null,
        stream: null,
        context,
        audio: new ConversationAudio(
          context,
          (playing) => {
            session.playing = playing;
            refreshPhase(session);
          },
          callbacksRef.current.volume,
        ),
        muted: false,
        deafened: false,
        changingAudio: false,
        hearing: false,
        turnDone: true,
        synthesisDone: true,
        playing: false,
        epoch: null,
        onset: null,
        timeout: null,
      };
      s = session;
      sessionRef.current = session;
      // A held Web Lock exempts Chromium from freezing this hidden webview.
      // Release it with the session, including failed or cancelled startup.
      void navigator.locks
        ?.request(
          "speakoflow-voice-conversation",
          { ifAvailable: true },
          async (lock) => {
            if (!lock || sessionRef.current !== session) return;
            await new Promise<void>((resolve) => {
              session.releaseBackgroundLock = resolve;
            });
          },
        )
        .catch(() => {});
      await context.resume();
      const acquire = async () => {
        let stream = await navigator.mediaDevices.getUserMedia({
          audio: {
            channelCount: 1,
            echoCancellation: true,
            noiseSuppression: true,
            autoGainControl: true,
          },
        });
        const wanted = callbacksRef.current.microphone;
        if (wanted && wanted !== "default") {
          const devices = await navigator.mediaDevices.enumerateDevices();
          const device = matchDeviceByName(devices, "audioinput", wanted);
          // No match is not a failure. The stored name comes from the recording
          // engine's own device enumeration, which is a different naming scheme
          // from the browser's labels — often identical on Windows, essentially
          // never on Linux — so demanding an exact match made the call
          // impossible to start for anyone who had picked a specific microphone.
          // The default stream is already open and is a working microphone.
          if (device) {
            stream.getTracks().forEach((t) => t.stop());
            stream = await navigator.mediaDevices.getUserMedia({
              audio: {
                deviceId: { exact: device.deviceId },
                channelCount: 1,
                echoCancellation: true,
                noiseSuppression: true,
                autoGainControl: true,
              },
            });
          } else {
            console.warn(
              `Voice conversation: microphone "${wanted}" was not found by the panel; using the system default.`,
            );
          }
        }
        if (!lifecycle.current.accepts(generation)) {
          stream.getTracks().forEach((t) => t.stop());
          throw new Error("Session ended");
        }
        session.stream = stream;
        stream.getAudioTracks().forEach((track) => {
          track.onended = () => {
            if (sessionRef.current === session && micLive(session))
              fail({ code: "device" });
          };
        });
        return stream;
      };
      await acquire();
      const output = callbacksRef.current.outputDevice;
      if (output && output !== "default") {
        // Best-effort, never fatal. `AudioContext.setSinkId` is Chromium-only
        // (absent on WebKitGTK and WKWebView), `enumerateDevices` does not list
        // outputs at all on WebKit, and the stored name is the playback engine's
        // rather than the browser's — so this used to abort the whole call on
        // macOS and Linux for anyone who had chosen an output device.
        try {
          const devices = await navigator.mediaDevices.enumerateDevices();
          const device = matchDeviceByName(devices, "audiooutput", output);
          const ctx = context as AudioContext & {
            setSinkId?: (id: string) => Promise<void>;
          };
          if (device && ctx.setSinkId) await ctx.setSinkId(device.deviceId);
          else
            console.warn(
              `Voice conversation: output device "${output}" is not selectable here; using the system default.`,
            );
        } catch (cause) {
          console.warn(
            "Voice conversation: could not set the output device",
            cause,
          );
        }
      }
      const { MicVAD } = await import("@ricky0123/vad-web");
      if (!lifecycle.current.accepts(generation)) {
        release(session);
        return;
      }
      await ready.current;
      // End must reach Rust before a retry starts. Also close a late start
      // before a newer start can run, even if permission/VAD setup was cancelled.
      const starting = backendLifecycle.current.then(async () => {
        if (!lifecycle.current.accepts(generation)) return null;
        const ticket = await invoke<VoiceTicket>(
          "assistant_conversation_start",
        );
        if (!lifecycle.current.accepts(generation)) {
          closedSessions.current.add(ticket.session);
          await invoke("assistant_conversation_end", {
            session: ticket.session,
          });
          return null;
        }
        session.id = ticket.session;
        return ticket;
      });
      backendLifecycle.current = starting.catch(() => {});
      const ticket = await starting;
      if (!ticket || !lifecycle.current.accepts(generation)) {
        release(session);
        return;
      }
      session.vad = await MicVAD.new({
        model: "v5",
        baseAssetPath: "/voice-assets/",
        onnxWASMBasePath: "/voice-assets/",
        audioContext: context,
        startOnLoad: true,
        processorType: "AudioWorklet",
        positiveSpeechThreshold: 0.65,
        negativeSpeechThreshold: 0.4,
        minSpeechMs: 160,
        preSpeechPadMs: 320,
        redemptionMs: TURN_PAUSE_MS[paceRef.current],
        submitUserSpeechOnPause: false,
        ortConfig: (ort) => {
          ort.env.wasm.numThreads = 1;
          ort.env.logLevel = "error";
        },
        getStream: async () => session.stream ?? acquire(),
        pauseStream: async (stream) => {
          stream.getTracks().forEach((track) => track.stop());
          session.stream = null;
        },
        resumeStream: acquire,
        onFrameProcessed: (_probability, frame) => {
          if (sessionRef.current !== session || !micLive(session)) return;
          // An open mic can run for hours. Keep room noise from re-rendering
          // the whole transcript on every 32 ms inference frame.
          if (!session.hearing) return;
          const now = performance.now();
          if (now - lastLevelAt.current < 75) return;
          lastLevelAt.current = now;
          let sum = 0;
          for (const sample of frame) sum += sample * sample;
          setLevel(Math.min(1, Math.sqrt(sum / frame.length) * 8));
        },
        onSpeechRealStart: () => {
          if (sessionRef.current !== session || !micLive(session)) return;
          session.hearing = true;
          setError(null);
          interrupt(session);
          refreshPhase(session);
          // A segment this long is either a monologue or a room the VAD never
          // hears fall silent (a fan, a TV). Either way it is one bad turn, not
          // a dead microphone: say so and keep listening. Ending the session
          // here released the mic mid-call and threw the speech away.
          session.timeout = setTimeout(
            () => recover({ code: "tooLong" }, session),
            MAX_UTTERANCE_MS,
          );
        },
        onVADMisfire: () => {
          if (sessionRef.current === session) {
            session.hearing = false;
            refreshPhase(session);
          }
        },
        onSpeechEnd: (audio) => {
          void submit(session, audio);
        },
      });
      if (!lifecycle.current.accepts(generation)) {
        release(session);
        return;
      }
      refreshPhase(session);
    } catch (cause) {
      if (!lifecycle.current.accepts(generation)) {
        if (s) release(s);
        return;
      }
      fail({
        code:
          cause instanceof DOMException &&
          ["NotAllowedError", "NotFoundError", "NotReadableError"].includes(
            cause.name,
          )
            ? "microphone"
            : "setup",
        detail: String(cause),
      });
    }
  }, [end, fail, interrupt, recover, refreshPhase, release, submit]);

  /**
   * Apply a change to the two audio switches and reconcile the session with it.
   *
   * One function for both buttons because the microphone is a function of both
   * (see `micLive`) and because only one VAD pause/start may be in flight at a
   * time. The asymmetry between them lives here and nowhere else: deafening
   * cancels the reply, muting deliberately leaves it running.
   */
  const setAudioState = useCallback(
    async (mutate: (s: Session) => void) => {
      const s = sessionRef.current;
      if (!s?.vad || s.changingAudio) return;
      const wasMuted = s.muted;
      const wasDeafened = s.deafened;
      const wasLive = micLive(s);
      mutate(s);
      if (s.muted === wasMuted && s.deafened === wasDeafened) return;
      setMuted(s.muted);
      setDeafened(s.deafened);
      if (!micLive(s) && wasLive) {
        // The microphone is closing, so an utterance in progress is abandoned.
        // Nothing to cancel on the backend: `submit` drops it before it is sent.
        s.hearing = false;
        if (s.timeout) clearTimeout(s.timeout);
        s.timeout = null;
      }
      // Deafening stops the answer as a barge-in does — there is no point
      // generating and synthesizing speech nobody can hear. Muting must not,
      // which is the whole reason the two are separate controls: the assistant
      // keeps talking while your microphone is off.
      if (s.deafened && !wasDeafened) interrupt(s);
      refreshPhase(s);
      setLevel(0);
      if (micLive(s) === wasLive) return;
      s.changingAudio = true;
      try {
        if (micLive(s)) await s.vad.start();
        else await s.vad.pause();
      } catch (cause) {
        // Reopening the microphone fails whenever another app has taken it in
        // the meantime. That is retryable: put the switches back and keep the
        // call alive so a second tap can succeed.
        if (sessionRef.current === s) {
          s.muted = wasMuted;
          s.deafened = wasDeafened;
          setMuted(wasMuted);
          setDeafened(wasDeafened);
          recover({ code: "microphone", detail: String(cause) }, s);
        }
      } finally {
        s.changingAudio = false;
      }
    },
    [interrupt, recover, refreshPhase],
  );

  /** Microphone off, speaker untouched: the assistant can still answer aloud. */
  const toggleMute = useCallback(
    () =>
      setAudioState((s) => {
        s.muted = !s.muted;
      }),
    [setAudioState],
  );

  /**
   * Both directions off — the headphones-down gesture. Turning it on also cuts
   * the reply in flight; turning it off restores whatever the microphone switch
   * was set to on its own, so deafening during a muted call does not silently
   * unmute you on the way back.
   */
  const toggleDeafen = useCallback(
    () =>
      setAudioState((s) => {
        s.deafened = !s.deafened;
      }),
    [setAudioState],
  );

  useEffect(() => {
    sessionRef.current?.vad?.setOptions({ redemptionMs: TURN_PAUSE_MS[pace] });
  }, [pace]);

  // Both dials reach a running session: the pace above, the volume here. A
  // setting changed mid-call should apply to that call.
  const volume = callbacks.volume;
  useEffect(() => {
    if (volume !== undefined) sessionRef.current?.audio.setVolume(volume);
  }, [volume]);

  useEffect(() => {
    let disposed = false;
    const unlisteners: UnlistenFn[] = [];
    const track = (unlisten: UnlistenFn) => {
      if (disposed) unlisten();
      else unlisteners.push(unlisten);
    };
    const current = (ticket: VoiceTicket) => {
      const s = sessionRef.current;
      return s && canHear(s) && !s.hearing && sameVoiceTicket(s.ticket, ticket)
        ? s
        : null;
    };
    ready.current = (async () => {
      track(
        await listen<{ ticket: VoiceTicket; epoch: number }>(
          "assistant-conversation-audio-begin",
          ({ payload }) => {
            const s = current(payload.ticket);
            if (!s) return;
            s.epoch = payload.epoch;
            s.synthesisDone = false;
            s.audio.begin(payload.epoch);
            refreshPhase(s);
          },
        ),
      );
      track(
        await listen<{ state: string }>("assistant-state", ({ payload }) => {
          const s = sessionRef.current;
          if (
            s?.ticket &&
            !s.hearing &&
            ["thinking", "searching"].includes(payload.state)
          )
            refreshPhase(s);
        }),
      );
      track(
        await listen<{ ticket: VoiceTicket; epoch: number; audio: string }>(
          "assistant-conversation-audio",
          ({ payload }) => {
            const s = current(payload.ticket);
            if (!s) return;
            s.epoch = payload.epoch;
            s.synthesisDone = false;
            s.audio.begin(payload.epoch);
            const bytes = Uint8Array.from(atob(payload.audio), (c) =>
              c.charCodeAt(0),
            );
            void s.audio.enqueue(bytes.buffer, payload.epoch).catch((cause) => {
              // One sentence that would not decode is not a broken speaker.
              // ConversationAudio already isolates the failure so later chunks
              // still play; tearing the call down over it lost the session.
              const session = current(payload.ticket);
              if (session)
                recover({ code: "playback", detail: String(cause) }, session);
            });
          },
        ),
      );
      track(
        await listen<{ ticket: VoiceTicket; epoch: number }>(
          "assistant-conversation-audio-end",
          ({ payload }) => {
            const s = current(payload.ticket);
            if (s && (s.epoch === null || s.epoch === payload.epoch)) {
              s.synthesisDone = true;
              refreshPhase(s);
            }
          },
        ),
      );
      track(
        await listen<{
          ticket: VoiceTicket;
          epoch: number;
          kind: string;
          text?: string;
        }>("assistant-conversation-local", ({ payload }) => {
          const s = current(payload.ticket);
          if (!s) return;
          if (payload.kind === "begin") {
            s.epoch = payload.epoch;
            s.synthesisDone = false;
            s.audio.begin(payload.epoch);
            void callbacksRef.current
              .beginLocal(payload.epoch)
              .catch((cause) => {
                const session = current(payload.ticket);
                if (session)
                  recover({ code: "playback", detail: String(cause) }, session);
              });
          } else if (payload.epoch === s.epoch) {
            if (payload.kind === "chunk")
              callbacksRef.current.pushLocal(payload.text ?? "");
            else callbacksRef.current.endLocal();
          }
        }),
      );
      track(
        await listen<number>("assistant-conversation-ended", ({ payload }) => {
          // A session this hook already tore down cannot end anything: its late
          // event must not reach the call that replaced it.
          if (closedSessions.current.has(payload)) return;
          const s = sessionRef.current;
          if (!s) return;
          // `id === 0` is a live session still waiting for its backend ticket.
          // Rust considers a call active from the moment it issues that ticket,
          // so an "ended" event can legitimately arrive before the id lands
          // here — and requiring the ids to match left the VAD holding the
          // microphone for a call the backend had already torn down, which is
          // what "the panel is using the mic and nothing works" looked like. The
          // in-flight start closes its own late ticket via the lifecycle
          // generation, so ending early is safe.
          if (s.id === payload || s.id === 0) end();
        }),
      );
      // A failed/pending start has no backend ticket to end. These explicit
      // surface changes must dismiss it too, or the call UI masks quick asks.
      track(await listen("assistant-panel-hidden", end));
      track(await listen("assistant-quick-ask", end));
      // Collapsing to the pill keeps the microphone, because the pill is on
      // screen and says the call is live. Hiding the panel does not: the backend
      // ends the session (see `assistant::hide_assistant_panel`) and this
      // listener tears the local side down with it.
      track(
        await listen<{ code: string; detail: string }>(
          "assistant-error",
          ({ payload }) => {
            const s = sessionRef.current;
            if (!s?.id) return;
            // `assistant-error` is a global event with background emitters (a
            // superseded turn's speech synthesis, TTS playback, screen capture).
            // Only treat it as *this* turn's failure when a turn is actually in
            // flight — otherwise a late error from work already abandoned marked
            // the live turn finished and flipped the orb back to "Listening"
            // while its reply was still being written.
            if (s.turnDone && s.synthesisDone) {
              setError({ code: "turn", detail: payload.detail });
              return;
            }
            recover({ code: "turn", detail: payload.detail }, s);
          },
        ),
      );
    })();
    const onKey = (event: KeyboardEvent) => {
      if (
        event.key === "Escape" &&
        !event.defaultPrevented &&
        openRef.current
      ) {
        event.preventDefault();
        // Escape on a call is a hang-up, so it has to take the window with it.
        // Ending only the session left the panel on screen as the quick-ask
        // card, which is the same glitch the End button had.
        const hangUp = callbacksRef.current.onHangUp;
        if (hangUp) hangUp();
        else end();
      }
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("pagehide", end);
    return () => {
      disposed = true;
      unlisteners.forEach((unlisten) => unlisten());
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("pagehide", end);
      end();
    };
  }, [end, fail, recover, refreshPhase]);

  const browserSink = useRef({
    enqueue: async (blob: Blob, epoch: number | null) => {
      const s = sessionRef.current;
      if (!s || epoch === null || s.epoch !== epoch || s.hearing || !canHear(s))
        return;
      await s.audio.enqueue(await blob.arrayBuffer(), epoch);
    },
    finish: (epoch: number | null) => {
      const s = sessionRef.current;
      if (s && s.epoch === epoch) {
        s.synthesisDone = true;
        refreshPhase(s);
      }
    },
  }).current;

  return {
    open,
    phase,
    error,
    level,
    pace,
    setPace,
    muted,
    deafened,
    start,
    end,
    toggleMute,
    toggleDeafen,
    browserSink,
  };
}
