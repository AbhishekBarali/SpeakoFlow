import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { MicVAD } from "@ricky0123/vad-web";
import { ConversationAudio } from "./conversationAudio";
import {
  BARGE_IN_DENSITY,
  DEFAULT_CONVERSATION_PACE,
  DEFAULT_CONVERSATION_SENSITIVITY,
  MAX_UTTERANCE_MS,
  matchDeviceByName,
  sameVoiceTicket,
  SPEECH_LEAD_HISTORY_FRAMES,
  speechLeadFrames,
  TURN_PAUSE_MS,
  VAD_FRAME_MS,
  VAD_SENSITIVITY,
  VoiceTurnGate,
  type ConversationPace,
  type ConversationSensitivity,
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
  code:
    | "microphone"
    | "device"
    | "setup"
    | "turn"
    | "playback"
    | "tooLong"
    | "history";
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
  /** The persisted sensitivity (`assistant_conversation_sensitivity`). */
  sensitivity?: ConversationSensitivity | null;
  onSensitivityChange: (sensitivity: ConversationSensitivity) => void;
  /**
   * Whether a new call reads its replies aloud. Where the call's speaker switch
   * starts; `assistant_tts_enabled` in the panel, and the backend seeds its own
   * copy from the same setting.
   */
  speakerOn?: boolean;
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

/**
 * Speech heard while the assistant is still talking, not yet allowed to cut it
 * off. `frames` is a sliding window of per-frame "was this speech" verdicts;
 * see `VAD_SENSITIVITY.bargeInMs` for why.
 */
interface PendingBargeIn {
  frames: boolean[];
}

/** One frame the detector has seen, kept for an utterance's lead-in. */
interface HeardFrame {
  frame: Float32Array;
  /** Speech probability, or `null` when the assistant's reply was playing. */
  probability: number | null;
}

/** Join audio chunks into one buffer, in order. */
const joinAudio = (parts: readonly Float32Array[]): Float32Array => {
  const out = new Float32Array(parts.reduce((sum, p) => sum + p.length, 0));
  let at = 0;
  for (const part of parts) {
    out.set(part, at);
    at += part.length;
  }
  return out;
};

interface Session {
  id: number;
  ticket: VoiceTicket | null;
  vad: MicVAD | null;
  stream: MediaStream | null;
  context: AudioContext;
  audio: ConversationAudio;
  /** Microphone off. Input only: a muted call still speaks and still types. */
  muted: boolean;
  /**
   * Replies are not read aloud. Output only: the microphone stays live.
   *
   * This replaced a "sound off" switch that closed the microphone as well,
   * which made the one control people reach for in a shared room also stop the
   * call from hearing them.
   */
  speakerOff: boolean;
  /** A VAD pause/start is in flight. */
  changingMic: boolean;
  /** An utterance is being captured as the next turn. */
  hearing: boolean;
  /** Speech over a playing reply that has not proven it is not echo. */
  pending: PendingBargeIn | null;
  /** The utterance in progress is to be dropped when it ends. */
  discard: boolean;
  /**
   * The frames heard since the last utterance was submitted, newest last and
   * capped at what a lead-in can use. See `SPEECH_LEAD`.
   */
  heard: HeardFrame[];
  /**
   * Audio from before the detector recognised the utterance in progress, put
   * back in front of it when it is submitted. `null` when nothing is pending.
   */
  lead: Float32Array | null;
  turnDone: boolean;
  synthesisDone: boolean;
  playing: boolean;
  epoch: number | null;
  onset: Promise<VoiceTicket> | null;
  timeout: ReturnType<typeof setTimeout> | null;
  releaseBackgroundLock?: () => void;
}

const micLive = (s: Session) => !s.muted;
const canHear = (s: Session) => !s.speakerOff;

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
   * it thinks and it speaks. Deriving a switch from the phase would force the
   * phase to mask "Thinking"/"Speaking" whenever the switch is down.
   */
  const [muted, setMuted] = useState(false);
  const [speakerOff, setSpeakerOff] = useState(false);
  const pace = callbacks.pace ?? DEFAULT_CONVERSATION_PACE;
  const setPace = callbacks.onPaceChange;
  const paceRef = useRef(pace);
  paceRef.current = pace;
  const sensitivity = callbacks.sensitivity ?? DEFAULT_CONVERSATION_SENSITIVITY;
  const setSensitivity = callbacks.onSensitivityChange;
  const sensitivityRef = useRef(sensitivity);
  sensitivityRef.current = sensitivity;
  /** The user is writing a message; keyboard noise must not start a turn. */
  const composingRef = useRef(false);
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
      s.hearing
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
    composingRef.current = false;
    setOpen(false);
    setPhase("off");
    setError(null);
    setLevel(0);
    setMuted(false);
    setSpeakerOff(false);
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
   * the session down. The next time they speak, the message clears.
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

  /** Forget the utterance in progress; the VAD's end of it will be dropped. */
  const abandonUtterance = useCallback((s: Session) => {
    if (s.hearing || s.pending) s.discard = true;
    s.hearing = false;
    s.pending = null;
    if (s.timeout) clearTimeout(s.timeout);
    s.timeout = null;
    setLevel(0);
  }, []);

  /** Stop whatever the assistant is saying locally, without a new turn. */
  const silence = useCallback((s: Session) => {
    s.epoch = null;
    s.audio.stop();
    callbacksRef.current.stopLocal();
  }, []);

  const interrupt = useCallback(
    (s: Session) => {
      const interruptedReply = s.playing || !s.synthesisDone;
      turns.current.next();
      s.ticket = null;
      silence(s);
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
    },
    [silence],
  );

  /** Speech is real and becomes the next turn: cut the reply off, listen. */
  const confirmSpeech = useCallback(
    (s: Session) => {
      s.pending = null;
      s.hearing = true;
      setError(null);
      interrupt(s);
      refreshPhase(s);
      if (s.timeout) clearTimeout(s.timeout);
      // A segment this long is either a monologue or a room the VAD never hears
      // fall silent (a fan, a TV). Either way it is one bad turn, not a dead
      // microphone: say so and keep listening.
      s.timeout = setTimeout(
        () => recover({ code: "tooLong" }, s),
        MAX_UTTERANCE_MS,
      );
    },
    [interrupt, recover, refreshPhase],
  );

  const submit = useCallback(
    async (s: Session, audio: Float32Array) => {
      if (sessionRef.current !== s || !micLive(s)) return;
      if (s.timeout) clearTimeout(s.timeout);
      s.timeout = null;
      s.hearing = false;
      const generation = turns.current.next();
      setLevel(0);
      try {
        // No onset means the turn this speech opened was replaced (a typed
        // message, New chat, a saved conversation) while it was being spoken.
        if (!s.onset) {
          refreshPhase(s);
          return;
        }
        setPhase("transcribing");
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
    const speakerStartsOff = callbacksRef.current.speakerOn === false;
    setSpeakerOff(speakerStartsOff);
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
        speakerOff: speakerStartsOff,
        changingMic: false,
        hearing: false,
        pending: null,
        discard: false,
        heard: [],
        lead: null,
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
      // Rust seeds its speaker switch from the same setting, but the switch may
      // have been flipped while the call was still connecting. One source of
      // truth: whatever this side shows.
      void invoke("assistant_conversation_set_speaker", {
        session: ticket.session,
        on: !session.speakerOff,
      }).catch(() => {});
      const tuning = VAD_SENSITIVITY[sensitivityRef.current];
      session.vad = await MicVAD.new({
        model: "v5",
        baseAssetPath: "/voice-assets/",
        onnxWASMBasePath: "/voice-assets/",
        audioContext: context,
        startOnLoad: true,
        processorType: "AudioWorklet",
        positiveSpeechThreshold: tuning.positiveSpeechThreshold,
        negativeSpeechThreshold: tuning.negativeSpeechThreshold,
        minSpeechMs: tuning.minSpeechMs,
        // Zero, because the lead-in is ours: the library's own pre-roll is a
        // fixed length, and a fixed length is what cut off first words. With it
        // at zero the audio the library hands back starts exactly at the frame
        // that opened the segment, which is what `onSpeechStart` below assumes.
        preSpeechPadMs: 0,
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
        onFrameProcessed: (probabilities, frame) => {
          if (sessionRef.current !== session || !micLive(session)) return;
          // Every frame, before anything below can return early: this is the
          // history a lead-in is cut from. Frames heard while a reply plays are
          // marked, so a lead-in never reaches back into the assistant's voice.
          session.heard.push({
            frame,
            probability: session.playing ? null : probabilities.isSpeech,
          });
          if (session.heard.length > SPEECH_LEAD_HISTORY_FRAMES)
            session.heard.shift();
          const pending = session.pending;
          if (pending) {
            // The echo guard. Speech that arrives while the assistant is
            // talking only interrupts once it has been speech-dense for a whole
            // `bargeInMs` window. Leaked playback comes through the echo
            // canceller in fragments and never gets there; a person who means
            // to interrupt keeps talking and does.
            const tuning = VAD_SENSITIVITY[sensitivityRef.current];
            const window = Math.max(
              1,
              Math.round(tuning.bargeInMs / VAD_FRAME_MS),
            );
            pending.frames.push(
              probabilities.isSpeech >= tuning.positiveSpeechThreshold,
            );
            if (pending.frames.length > window) pending.frames.shift();
            const speech = pending.frames.filter(Boolean).length;
            if (
              pending.frames.length >= window &&
              speech / window >= BARGE_IN_DENSITY
            )
              confirmSpeech(session);
            return;
          }
          // An open mic can run for hours. Keep room noise from re-rendering
          // the whole panel on every 32 ms inference frame.
          if (!session.hearing) return;
          const now = performance.now();
          if (now - lastLevelAt.current < 75) return;
          lastLevelAt.current = now;
          let sum = 0;
          for (const sample of frame) sum += sample * sample;
          setLevel(Math.min(1, Math.sqrt(sum / frame.length) * 8));
        },
        onSpeechStart: () => {
          if (sessionRef.current !== session || !micLive(session)) return;
          // The frame that opened the segment is the newest one heard (the
          // library reports a frame before it reports what that frame started),
          // and the library's audio begins with it. Everything before it is
          // candidate lead-in.
          const before = session.heard.slice(0, -1);
          const count = speechLeadFrames(before.map((f) => f.probability));
          session.lead =
            count > 0
              ? joinAudio(
                  before.slice(before.length - count).map((f) => f.frame),
                )
              : null;
        },
        onSpeechRealStart: () => {
          if (sessionRef.current !== session || !micLive(session)) return;
          // Typing makes noise the detector hears as speech. A message being
          // written is the user's turn already.
          if (composingRef.current) {
            session.discard = true;
            return;
          }
          if (session.playing) {
            session.pending = { frames: [] };
            return;
          }
          confirmSpeech(session);
        },
        onVADMisfire: () => {
          if (sessionRef.current !== session) return;
          session.hearing = false;
          session.pending = null;
          session.discard = false;
          // The frames stay in `heard`: a short sound the detector rejected
          // ("hey", then a pause) is exactly what the next utterance's lead-in
          // may need to reach back to.
          session.lead = null;
          refreshPhase(session);
        },
        onSpeechEnd: (audio) => {
          if (sessionRef.current !== session) return;
          const lead = session.lead;
          session.lead = null;
          // Submitted or dropped, this audio is spoken for: a later lead-in must
          // not reach back into it and repeat its words.
          session.heard = [];
          // Speech that never earned the right to interrupt is dropped rather
          // than answered: it was most likely the assistant hearing itself.
          if (session.discard || session.pending) {
            session.discard = false;
            session.pending = null;
            refreshPhase(session);
            return;
          }
          void submit(session, lead ? joinAudio([lead, audio]) : audio);
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
  }, [confirmSpeech, end, fail, refreshPhase, release, submit]);

  /** Microphone on or off. The speaker and the reply in flight are untouched. */
  const toggleMute = useCallback(async () => {
    const s = sessionRef.current;
    if (!s?.vad || s.changingMic) return;
    const next = !s.muted;
    s.muted = next;
    setMuted(next);
    if (next) {
      // The microphone is closing, so an utterance in progress is abandoned.
      // Nothing to cancel on the backend: `submit` drops it before it is sent.
      s.hearing = false;
      s.pending = null;
      s.discard = false;
      // Nothing heard before the mute may open the first utterance after it.
      s.heard = [];
      s.lead = null;
      if (s.timeout) clearTimeout(s.timeout);
      s.timeout = null;
    }
    refreshPhase(s);
    setLevel(0);
    s.changingMic = true;
    try {
      if (next) await s.vad.pause();
      else await s.vad.start();
    } catch (cause) {
      // Reopening the microphone fails whenever another app has taken it in
      // the meantime. That is retryable: put the switch back and keep the call
      // alive so a second tap can succeed.
      if (sessionRef.current === s) {
        s.muted = !next;
        setMuted(!next);
        recover({ code: "microphone", detail: String(cause) }, s);
      }
    } finally {
      s.changingMic = false;
    }
  }, [recover, refreshPhase]);

  /**
   * Replies read aloud, or not. Turning it off stops what is being said right
   * now and lets the reply finish as text; the microphone stays exactly as the
   * mute button left it.
   */
  const toggleSpeaker = useCallback(async () => {
    const s = sessionRef.current;
    if (!s) return;
    const off = !s.speakerOff;
    s.speakerOff = off;
    setSpeakerOff(off);
    if (off) {
      silence(s);
      s.synthesisDone = true;
      // Nothing is playing any more, so nothing can be echo.
      if (s.pending) confirmSpeech(s);
    }
    refreshPhase(s);
    if (s.id)
      await invoke("assistant_conversation_set_speaker", {
        session: s.id,
        on: !off,
      }).catch(() => {});
  }, [confirmSpeech, refreshPhase, silence]);

  /**
   * Send a typed message as the next turn. Resolves `true` once the reply has
   * finished (or was superseded), `false` when it could not be sent.
   */
  const sendText = useCallback(
    async (text: string): Promise<boolean> => {
      const s = sessionRef.current;
      const message = text.trim();
      if (!s?.id || !message) return false;
      // Speech in progress belonged to the turn this message replaces.
      abandonUtterance(s);
      const onset = interrupt(s);
      // The spoken turn that onset would have carried is gone.
      s.onset = null;
      const generation = turns.current.next();
      setError(null);
      refreshPhase(s);
      try {
        const ticket = await onset;
        if (sessionRef.current !== s || !turns.current.accepts(generation))
          return false;
        s.ticket = ticket;
        s.turnDone = false;
        s.synthesisDone = true;
        refreshPhase(s);
        await invoke("assistant_conversation_text", {
          session: ticket.session,
          turn: ticket.turn,
          text: message,
        });
        if (sessionRef.current === s && turns.current.accepts(generation)) {
          s.turnDone = true;
          refreshPhase(s);
        }
        return true;
      } catch (cause) {
        if (sessionRef.current !== s || !turns.current.accepts(generation))
          return false;
        recover({ code: "turn", detail: String(cause) }, s);
        return false;
      }
    },
    [abandonUtterance, interrupt, recover, refreshPhase],
  );

  /** Stop the reply the assistant is giving, and keep the call listening. */
  const stopReply = useCallback(() => {
    const s = sessionRef.current;
    if (!s?.id) return;
    if (s.turnDone && s.synthesisDone && !s.playing) return;
    s.pending = null;
    interrupt(s);
    refreshPhase(s);
  }, [interrupt, refreshPhase]);

  /**
   * Replace the call's conversation — with a new one, or with one from
   * History — without hanging up.
   */
  const switchConversation = useCallback(
    async (
      command: "assistant_conversation_new" | "assistant_conversation_load",
      args: Record<string, number>,
    ): Promise<boolean> => {
      const s = sessionRef.current;
      if (!s?.id) return false;
      turns.current.next();
      abandonUtterance(s);
      s.ticket = null;
      s.onset = null;
      silence(s);
      s.turnDone = true;
      s.synthesisDone = true;
      setError(null);
      refreshPhase(s);
      try {
        await invoke(command, { session: s.id, ...args });
        return true;
      } catch (cause) {
        if (sessionRef.current === s)
          setError({ code: "history", detail: String(cause) });
        return false;
      }
    },
    [abandonUtterance, refreshPhase, silence],
  );
  const newConversation = useCallback(
    () => switchConversation("assistant_conversation_new", {}),
    [switchConversation],
  );
  const loadConversation = useCallback(
    (id: number) => switchConversation("assistant_conversation_load", { id }),
    [switchConversation],
  );

  /** Whether a message is being written (see `composingRef`). */
  const setComposing = useCallback((composing: boolean) => {
    composingRef.current = composing;
  }, []);

  const clearError = useCallback(() => setError(null), []);

  useEffect(() => {
    sessionRef.current?.vad?.setOptions({ redemptionMs: TURN_PAUSE_MS[pace] });
  }, [pace]);

  useEffect(() => {
    const tuning = VAD_SENSITIVITY[sensitivity];
    sessionRef.current?.vad?.setOptions({
      positiveSpeechThreshold: tuning.positiveSpeechThreshold,
      negativeSpeechThreshold: tuning.negativeSpeechThreshold,
      minSpeechMs: tuning.minSpeechMs,
    });
  }, [sensitivity]);

  // Every dial reaches a running session: the pace and sensitivity above, the
  // volume here. A setting changed mid-call should apply to that call.
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
          // microphone for a call the backend had already torn down. The
          // in-flight start closes its own late ticket via the lifecycle
          // generation, so ending early is safe.
          if (s.id === payload || s.id === 0) end();
        }),
      );
      // A failed/pending start has no backend ticket to end. These explicit
      // surface changes must dismiss it too, or the call UI masks quick asks.
      track(await listen("assistant-panel-hidden", end));
      track(await listen("assistant-quick-ask", end));
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
            // the live turn finished while its reply was still being written.
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
    clearError,
    level,
    pace,
    setPace,
    sensitivity,
    setSensitivity,
    muted,
    speakerOff,
    start,
    end,
    toggleMute,
    toggleSpeaker,
    sendText,
    stopReply,
    newConversation,
    loadConversation,
    setComposing,
    browserSink,
  };
}
