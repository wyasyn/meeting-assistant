import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AppError, listAppRules, setAppRule } from "@/lib/ipc";
import { AppRules } from "./AppRules";

vi.mock("@/lib/ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/ipc")>()),
  listAppRules: vi.fn(),
  setAppRule: vi.fn(),
}));

const listMock = vi.mocked(listAppRules);
const setMock = vi.mocked(setAppRule);

beforeEach(() => {
  vi.resetAllMocks();
  listMock.mockResolvedValue([
    { sourceApp: "zoom", rule: "ask" },
    { sourceApp: "browser", rule: "never" },
  ]);
});

describe("AppRules (FR-1.7)", () => {
  it("shows each app with its rule", async () => {
    render(<AppRules />);
    expect(await screen.findByRole("combobox", { name: "Zoom" })).toHaveValue("ask");
    expect(screen.getByRole("combobox", { name: "Browser" })).toHaveValue("never");
  });

  it("saves a change", async () => {
    setMock.mockResolvedValue(undefined);
    render(<AppRules />);
    const zoom = await screen.findByRole("combobox", { name: "Zoom" });
    await userEvent.selectOptions(zoom, "Always record");
    expect(setMock).toHaveBeenCalledWith("zoom", "always");
    expect(zoom).toHaveValue("always");
  });

  it("shows a readable message when saving fails", async () => {
    setMock.mockRejectedValue(
      new AppError({ code: "storage", message: "Could not save.", retryable: false }),
    );
    render(<AppRules />);
    const zoom = await screen.findByRole("combobox", { name: "Zoom" });
    await userEvent.selectOptions(zoom, "Never record");
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not save.");
    expect(zoom).toHaveValue("ask");
  });
});
