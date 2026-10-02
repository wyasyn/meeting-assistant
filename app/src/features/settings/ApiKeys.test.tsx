import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AppError, clearApiKey, listProviders, setApiKey, testProvider } from "@/lib/ipc";
import { ApiKeys } from "./ApiKeys";

vi.mock("@/lib/ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/ipc")>()),
  listProviders: vi.fn(),
  setApiKey: vi.fn(),
  clearApiKey: vi.fn(),
  testProvider: vi.fn(),
}));

const listMock = vi.mocked(listProviders);
const setMock = vi.mocked(setApiKey);
const clearMock = vi.mocked(clearApiKey);
const testMock = vi.mocked(testProvider);

beforeEach(() => {
  vi.resetAllMocks();
  listMock.mockResolvedValue([{ provider: "gemini", hasKey: false }]);
});

describe("ApiKeys (FR-8.1)", () => {
  it("saves a pasted key and clears the field", async () => {
    setMock.mockResolvedValue(undefined);
    render(<ApiKeys />);
    expect(await screen.findByText("No key saved")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Test key" })).toBeDisabled();

    const input = screen.getByLabelText("Gemini API key");
    await userEvent.type(input, "abc123");
    await userEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(setMock).toHaveBeenCalledWith("gemini", "abc123");
    expect(await screen.findByText("Key saved")).toBeInTheDocument();
    expect(input).toHaveValue("");
    expect(screen.getByRole("button", { name: "Test key" })).toBeEnabled();
  });

  it("shows the test result", async () => {
    listMock.mockResolvedValue([{ provider: "gemini", hasKey: true }]);
    testMock.mockResolvedValue({ ok: false, message: "Google did not accept this API key." });
    render(<ApiKeys />);
    await userEvent.click(await screen.findByRole("button", { name: "Test key" }));
    expect(testMock).toHaveBeenCalledWith("gemini");
    expect(await screen.findByRole("status")).toHaveTextContent(
      "Google did not accept this API key.",
    );
  });

  it("removes a saved key", async () => {
    listMock.mockResolvedValue([{ provider: "gemini", hasKey: true }]);
    clearMock.mockResolvedValue(undefined);
    render(<ApiKeys />);
    await userEvent.click(await screen.findByRole("button", { name: "Remove key" }));
    expect(clearMock).toHaveBeenCalledWith("gemini");
    expect(await screen.findByText("No key saved")).toBeInTheDocument();
  });

  it("shows a readable message when the keychain fails", async () => {
    setMock.mockRejectedValue(
      new AppError({ code: "storage", message: "Could not reach the keychain.", retryable: false }),
    );
    render(<ApiKeys />);
    await userEvent.type(await screen.findByLabelText("Gemini API key"), "abc");
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not reach the keychain.");
    expect(screen.getByText("No key saved")).toBeInTheDocument();
  });
});
