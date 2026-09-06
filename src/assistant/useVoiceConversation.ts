import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { MicVAD } from "@ricky0123/vad-web";
import { ConversationAudio } from "./conversationAudio";
import {
  MAX_UTTERANCE_MS,
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
  const [pace, setPace] = useState<ConversationPace>("natural");
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
          const device = devices.find(
            (d) => d.kind === "audioinput" && d.label === wanted,
          );
          if (!device) {
            stream.getTracks().forEach((t) => t.stop());
            throw new Error("Selected microphone is unavailable");
          }
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
        const devices = await navigator.mediaDevices.enumerateDevices();
        const device = devices.find(
          (d) => d.kind === "audiooutput" && d.label === output,
        );
        const ctx = context as AudioContext & {
          setSinkId?: (id: string) => Promise<void>;
        };
        if (!device || !ctx.setSinkId)
          throw new Error(
            "Selected output device is unavailable for conversation. Choose the system default in Settings.",
          );
        await ctx.setSinkId(device.deviceId);
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
          session.timeout = setTimeout(
            () => fail({ code: "tooLong" }),
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
  }, [end, fail, interrupt, refreshPhase, release, submit]);

  const toggleMute = useCallback(async () => {
    const s = sessionRef.current;
    if (!s?.vad || s.changingMic) return;
    s.changingMic = true;
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
      if (sessionRef.current === s)
        fail({ code: "microphone", detail: String(cause) });
    } finally {
      s.changingMic = false;
    }
  }, [fail, interrupt, refreshPhase]);

  useEffect(() => {
    sessionRef.current?.vad?.setOptions({ redemptionMs: TURN_PAUSE_MS[pace] });
  }, [pace]);

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
              if (current(payload.ticket))
                fail({ code: "playback", detail: String(cause) });
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
                if (current(payload.ticket))
                  fail({ code: "playback", detail: String(cause) });
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
      // Visibility is independent of the session: a hidden or collapsed panel
      // still owns the microphone until End (or actual webview teardown).
      track(
        await listen<{ code: string; detail: string }>(
          "assistant-error",
          ({ payload }) => {
            const s = sessionRef.current;
            if (s?.id) recover({ code: "turn", detail: payload.detail }, s);
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
