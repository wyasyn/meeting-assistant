import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AppError, getSettings, setSettings } from "@/lib/ipc";
import { StartOnLoginToggle } from "./StartOnLoginToggle";

vi.mock("@/lib/ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/ipc")>()),
  getSettings: vi.fn(),
  setSettings: vi.fn(),
}));

const getMock = vi.mocked(getSettings);
const setMock = vi.mocked(setSettings);

beforeEach(() => {
  vi.resetAllMocks();
});

describe("StartOnLoginToggle (FR-8.5)", () => {
  it("shows the current state from the core", async () => {
    getMock.mockResolvedValue({ startOnLogin: true });
    render(<StartOnLoginToggle />);
    expect(await screen.findByRole("checkbox", { name: "Start on login" })).toBeChecked();
  });

  it("saves a change and shows the state the core returns", async () => {
    getMock.mockResolvedValue({ startOnLogin: false });
    setMock.mockResolvedValue({ startOnLogin: true });
    render(<StartOnLoginToggle />);
    const box = await screen.findByRole("checkbox", { name: "Start on login" });
    await vi.waitFor(() => {
      expect(box).toBeEnabled();
    });

    await userEvent.click(box);
    expect(setMock).toHaveBeenCalledWith({ startOnLogin: true });
    expect(box).toBeChecked();
  });

  it("shows a readable message when saving fails", async () => {
    getMock.mockResolvedValue({ startOnLogin: false });
    setMock.mockRejectedValue(
      new AppError({ code: "internal", message: "Could not change it.", retryable: false }),
    );
    render(<StartOnLoginToggle />);
    const box = await screen.findByRole("checkbox", { name: "Start on login" });
    await vi.waitFor(() => {
      expect(box).toBeEnabled();
    });

    await userEvent.click(box);
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not change it.");
    expect(box).not.toBeChecked();
  });
});
