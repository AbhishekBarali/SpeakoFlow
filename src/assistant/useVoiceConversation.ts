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
}
interface Session {
  id: number;
  ticket: VoiceTicket | null;
  vad: MicVAD | null;
  stream: MediaStream | null;
  context: AudioContext;
  audio: ConversationAudio;
  muted: boolean;
  changingMic: boolean;
  hearing: boolean;
  turnDone: boolean;
  synthesisDone: boolean;
  playing: boolean;
  epoch: number | null;
  onset: Promise<VoiceTicket> | null;
  timeout: ReturnType<typeof setTimeout> | null;
  releaseBackgroundLock?: () => void;
}

export function useVoiceConversation(callbacks: VoiceCallbacks) {
  const callbacksRef = useRef(callbacks);
  callbacksRef.current = callbacks;
  const [open, setOpen] = useState(false);
  const [phase, setPhase] = useState<ConversationPhase>("off");
  const [error, setError] = useState<VoiceError | null>(null);
  const [level, setLevel] = useState(0);
  const pace = callbacks.pace ?? DEFAULT_CONVERSATION_PACE;
  const setPace = callbacks.onPaceChange;
  const paceRef = useRef(pace);
  paceRef.current = pace;
  const sessionRef = useRef<Session | null>(null);
  const lifecycle = useRef(new VoiceTurnGate());
  const turns = useRef(new VoiceTurnGate());
  const ready = useRef<Promise<unknown>>(Promise.resolve());
  const lastLevelAt = useRef(0);

  const refreshPhase = useCallback((s: Session) => {
    if (sessionRef.current !== s) return;
    setPhase(
      s.muted
        ? "muted"
        : s.hearing
          ? "hearing"
          : s.playing
            ? "speaking"
            : !s.turnDone || !s.synthesisDone
              ? "responding"
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
    if (s.id)
      void invoke("assistant_conversation_end", { session: s.id }).catch(
        () => {},
      );
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
    setLevel(0);
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
      if (sessionRef.current !== s || s.muted) return;
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
          s.muted
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
        changingMic: false,
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
            if (sessionRef.current === session && !session.muted)
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
      const ticket = await invoke<VoiceTicket>("assistant_conversation_start");
      session.id = ticket.session;
      if (!lifecycle.current.accepts(generation)) {
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
          if (sessionRef.current !== session || session.muted) return;
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
          if (sessionRef.current !== session || session.muted) return;
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

  const toggleMute = useCallback(async () => {
    const s = sessionRef.current;
    if (!s?.vad || s.changingMic) return;
    s.changingMic = true;
    const wasMuted = s.muted;
    s.muted = !s.muted;
    s.hearing = false;
    if (s.timeout) clearTimeout(s.timeout);
    s.timeout = null;
    interrupt(s);
    refreshPhase(s);
    setLevel(0);
    try {
      if (s.muted) await s.vad.pause();
      else await s.vad.start();
    } catch (cause) {
      // Unmuting reopens the microphone, so it fails whenever another app has
      // taken it in the meantime. That is a retryable condition: put the mute
      // state back and keep the call alive so a second tap can succeed.
      if (sessionRef.current === s) {
        s.muted = wasMuted;
        recover({ code: "microphone", detail: String(cause) }, s);
      }
    } finally {
      s.changingMic = false;
    }
  }, [interrupt, recover, refreshPhase]);

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
      return s && !s.muted && !s.hearing && sameVoiceTicket(s.ticket, ticket)
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
          if (sessionRef.current?.id === payload) end();
        }),
      );
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
        sessionRef.current
      ) {
        event.preventDefault();
        end();
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
      if (!s || epoch === null || s.epoch !== epoch || s.hearing || s.muted)
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
    start,
    end,
    toggleMute,
    browserSink,
  };
}
