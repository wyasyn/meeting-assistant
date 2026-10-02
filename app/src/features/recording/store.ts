import { create } from "zustand";
import {
  getRecordingState,
  onRecordingLevels,
  onRecordingState,
  toAppError,
  type RecordingLevels,
  type RecordingState,
} from "@/lib/ipc";

interface RecordingStore {
  recording: RecordingState;
  /** `Date.now()` when `recording` arrived; the timer counts on from there. */
  receivedAt: number;
  levels: RecordingLevels | null;
  /** Could not reach the core for the state. */
  loadError: string | null;
  apply: (recording: RecordingState) => void;
  setLevels: (levels: RecordingLevels) => void;
}

const IDLE: RecordingState = { meetingId: null, state: "idle", elapsedMs: 0, error: null };

export const useRecordingStore = create<RecordingStore>()((set) => ({
  recording: IDLE,
  receivedAt: Date.now(),
  levels: null,
  loadError: null,
  apply: (recording) => {
    set({
      recording,
      receivedAt: Date.now(),
      loadError: null,
      // Meters only make sense while audio is flowing.
      ...(recording.state === "recording" ? {} : { levels: null }),
    });
  },
  setLevels: (levels) => {
    set({ levels });
  },
}));

/** Loads the current state and follows `recording:state` / `recording:levels`. Returns cleanup. */
export function initRecording(): () => void {
  const { apply, setLevels } = useRecordingStore.getState();
  const unlisteners = [
    onRecordingState(apply),
    onRecordingLevels((levels) => {
      if (useRecordingStore.getState().recording.state === "recording") setLevels(levels);
    }),
  ];
  getRecordingState()
    .then(apply)
    .catch((e: unknown) => {
      useRecordingStore.setState({ loadError: toAppError(e).message });
    });
  return () => {
    for (const unlisten of unlisteners) {
      void unlisten.then((fn) => {
        fn();
      });
    }
  };
}
