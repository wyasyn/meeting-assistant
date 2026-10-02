import { create } from "zustand";
import {
  onMeetingDetected,
  onMeetingEnded,
  onMeetingPromptClosed,
  type MeetingDetected,
  type MeetingEnded,
} from "@/lib/ipc";

interface ConsentStore {
  /** Open consent prompts, oldest first. The core sends at most one per app. */
  prompts: MeetingDetected[];
  /** The "Meeting over?" prompt for the recording (FR-1.6); one at a time. */
  ending: MeetingEnded | null;
  add: (signal: MeetingDetected) => void;
  setEnding: (ended: MeetingEnded) => void;
  /** Removes the consent or end prompt with this id. */
  remove: (signalId: string) => void;
}

export const useConsentStore = create<ConsentStore>()((set) => ({
  prompts: [],
  ending: null,
  add: (signal) => {
    set((s) => ({
      prompts: [...s.prompts.filter((p) => p.sourceApp !== signal.sourceApp), signal],
    }));
  },
  setEnding: (ending) => {
    set({ ending });
  },
  remove: (signalId) => {
    set((s) => ({
      prompts: s.prompts.filter((p) => p.signalId !== signalId),
      ending: s.ending?.signalId === signalId ? null : s.ending,
    }));
  },
}));

/** Follows `meeting:detected` / `meeting:ended` / `meeting:prompt-closed`. Returns cleanup. */
export function initConsent(): () => void {
  const { add, setEnding, remove } = useConsentStore.getState();
  const unlisteners = [
    onMeetingDetected(add),
    onMeetingEnded(setEnding),
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
