import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AppError, setAppRule, startRecording, type MeetingDetected } from "@/lib/ipc";
import { ConsentPrompts } from "./ConsentPrompts";
import { useConsentStore } from "./store";

vi.mock("@/lib/ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/ipc")>()),
  startRecording: vi.fn(),
  setAppRule: vi.fn(),
}));

const startMock = vi.mocked(startRecording);
const ruleMock = vi.mocked(setAppRule);

function signal(signalId: string, sourceApp: MeetingDetected["sourceApp"]): MeetingDetected {
  return { signalId, sourceApp, title: null, confidence: 0.9 };
}

beforeEach(() => {
  vi.resetAllMocks();
  useConsentStore.setState({ prompts: [] });
});

describe("ConsentPrompts (FR-1.3, FR-1.7)", () => {
  it("shows nothing until a meeting is detected", () => {
    const { container } = render(<ConsentPrompts />);
    expect(container).toBeEmptyDOMElement();
  });

  it("records only when Record is clicked, then closes", async () => {
    useConsentStore.getState().add(signal("s1", "zoom"));
    startMock.mockResolvedValue({
      id: "m1",
      title: "Zoom meeting",
      sourceApp: "zoom",
      startedAt: 0,
      endedAt: null,
      durationS: null,
      status: "recording",
    });
    render(<ConsentPrompts />);
    expect(screen.getByText("Zoom is using your microphone.")).toBeInTheDocument();
    expect(startMock).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole("button", { name: "Record" }));
    expect(startMock).toHaveBeenCalledWith({ sourceApp: "zoom" });
    expect(screen.queryByRole("region", { name: "Record this meeting?" })).toBeNull();
  });

  it("Not now closes without recording", async () => {
    useConsentStore.getState().add(signal("s1", "slack"));
    render(<ConsentPrompts />);
    await userEvent.click(screen.getByRole("button", { name: "Not now" }));
    expect(startMock).not.toHaveBeenCalled();
    expect(ruleMock).not.toHaveBeenCalled();
    expect(useConsentStore.getState().prompts).toEqual([]);
  });

  it("Never sets the app rule", async () => {
    useConsentStore.getState().add(signal("s1", "browser"));
    ruleMock.mockResolvedValue(undefined);
    render(<ConsentPrompts />);
    await userEvent.click(screen.getByRole("button", { name: "Never for browsers" }));
    expect(ruleMock).toHaveBeenCalledWith("browser", "never");
    expect(useConsentStore.getState().prompts).toEqual([]);
  });

  it("keeps the prompt and says why when recording cannot start", async () => {
    useConsentStore.getState().add(signal("s1", "teams"));
    startMock.mockRejectedValue(
      new AppError({ code: "audio_device", message: "No microphone found.", retryable: false }),
    );
    render(<ConsentPrompts />);
    await userEvent.click(screen.getByRole("button", { name: "Record" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("No microphone found.");
    expect(screen.getByRole("button", { name: "Record" })).toBeEnabled();
  });

  it("keeps one prompt per app and drops answered ones", () => {
    const { add, remove } = useConsentStore.getState();
    add(signal("s1", "zoom"));
    add(signal("s2", "slack"));
    add(signal("s3", "zoom"));
    expect(useConsentStore.getState().prompts.map((p) => p.signalId)).toEqual(["s2", "s3"]);
    remove("s2");
    expect(useConsentStore.getState().prompts.map((p) => p.signalId)).toEqual(["s3"]);
  });
});
