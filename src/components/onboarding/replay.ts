import { create } from "zustand";

/**
 * Settings → General → "Show onboarding again".
 *
 * The Settings dialog lives inside the main shell, and onboarding replaces the
 * shell, so the request has to travel up to `App` without threading a
 * callback through every page. A counter rather than a flag, so pressing it a
 * second time (after finishing the first replay) is still a change `App` sees.
 */
interface OnboardingReplayState {
  requests: number;
  request: () => void;
}

export const useOnboardingReplay = create<OnboardingReplayState>()((set) => ({
  requests: 0,
  request: () => set((state) => ({ requests: state.requests + 1 })),
}));
