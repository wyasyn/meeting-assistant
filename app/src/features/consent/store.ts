import { create } from "zustand";
import { onMeetingDetected, onMeetingPromptClosed, type MeetingDetected } from "@/lib/ipc";

interface ConsentStore {
  /** Open consent prompts, oldest first. The core sends at most one per app. */
  prompts: MeetingDetected[];
  add: (signal: MeetingDetected) => void;
  remove: (signalId: string) => void;
}

export const useConsentStore = create<ConsentStore>()((set) => ({
  prompts: [],
  add: (signal) => {
    set((s) => ({
      prompts: [...s.prompts.filter((p) => p.sourceApp !== signal.sourceApp), signal],
    }));
  },
  remove: (signalId) => {
    set((s) => ({ prompts: s.prompts.filter((p) => p.signalId !== signalId) }));
  },
}));

/** Follows `meeting:detected` / `meeting:prompt-closed` (FR-1.3). Returns cleanup. */
export function initConsent(): () => void {
  const { add, remove } = useConsentStore.getState();
  const unlisteners = [
    onMeetingDetected(add),
    onMeetingPromptClosed(({ signalId }) => {
      remove(signalId);
    }),
  ];
  return () => {
    for (const unlisten of unlisteners) {
      void unlisten.then((fn) => {
        fn();
      });
    }
  };
}
