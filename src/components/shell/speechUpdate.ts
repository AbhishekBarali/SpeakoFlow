import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import { useNavigation } from "./navigation";

/**
 * The backend's "open the speech model update" request, and the one-shot
 * scroll it asks of the prompt.
 *
 * A voice shortcut pressed while the selected model is retired is refused
 * (`speech_readiness.rs`), and besides the overlay's short notice the main
 * window comes forward on the Models page's speech tab, where the prompt that
 * downloads the new version sits at the top. The page may have been left
 * scrolled down, and may not be mounted yet when the request arrives, so the
 * request is a counter the prompt compares against what it last answered
 * rather than an event it has to be listening for at the time.
 */

/** Must match `speech_readiness::SPEECH_UPDATE_EVENT`. */
export const SPEECH_UPDATE_EVENT = "speech-model-update-needed";

interface SpeechUpdateReveal {
  /** Requests received. */
  requested: number;
  /** Requests a prompt has scrolled into view for. */
  answered: number;
  /** The update pop-up was closed with "Later" (or Escape). It stays closed
   *  for this run of the app, until a dictation is refused for the same
   *  reason, which is a request and opens it again. */
  dialogClosed: boolean;
  request: () => void;
  /** Claim the pending request, if any. True when the caller should scroll. */
  answer: () => boolean;
  closeDialog: () => void;
}

export const useSpeechUpdateReveal = create<SpeechUpdateReveal>((set, get) => ({
  requested: 0,
  answered: 0,
  dialogClosed: false,
  request: () =>
    set((state) => ({ requested: state.requested + 1, dialogClosed: false })),
  closeDialog: () => set({ dialogClosed: true }),
  answer: () => {
    const { requested, answered } = get();
    if (requested === answered) return false;
    set({ answered: requested });
    return true;
  },
}));

/** Shell effect: follow the backend's request to the speech tab. */
export const useSpeechUpdateRequests = (): void => {
  const { openModelSlot } = useNavigation();
  useEffect(() => {
    const unlisten = listen(SPEECH_UPDATE_EVENT, () => {
      openModelSlot("stt");
      useSpeechUpdateReveal.getState().request();
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [openModelSlot]);
};
