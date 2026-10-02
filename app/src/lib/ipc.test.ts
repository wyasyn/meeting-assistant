import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AppError, INTERNAL_MESSAGE, call, isAppErrorPayload, on } from "./ipc";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

const invokeMock = vi.mocked(invoke);
const listenMock = vi.mocked(listen);

beforeEach(() => {
  vi.resetAllMocks();
});

describe("call", () => {
  it("resolves with the command result", async () => {
    invokeMock.mockResolvedValue({ id: "m1" });
    await expect(call("get_meeting", { id: "m1" })).resolves.toEqual({ id: "m1" });
    expect(invokeMock).toHaveBeenCalledWith("get_meeting", { id: "m1" });
  });

  it("rejects with the AppError sent by the core", async () => {
    invokeMock.mockRejectedValue({
      code: "sidecar_down",
      message: "The speech service stopped.",
      retryable: true,
    });
    const error = await call("search").catch((e: unknown) => e);
    expect(error).toBeInstanceOf(AppError);
    expect(error).toMatchObject({
      code: "sidecar_down",
      message: "The speech service stopped.",
      retryable: true,
    });
  });

  it.each([
    ["a plain string", "command search not found"],
    ["an unknown code", { code: "boom", message: "x", retryable: false }],
    ["null", null],
  ])("turns %s into an internal AppError", async (_label, rejection) => {
    invokeMock.mockRejectedValue(rejection);
    const error = await call("search").catch((e: unknown) => e);
    expect(error).toBeInstanceOf(AppError);
    expect(error).toMatchObject({
      code: "internal",
      message: INTERNAL_MESSAGE,
      retryable: false,
      original: rejection,
    });
  });
});

describe("isAppErrorPayload", () => {
  it("rejects a payload with a wrong field type", () => {
    expect(isAppErrorPayload({ code: "storage", message: "x", retryable: "no" })).toBe(false);
  });
});

describe("on", () => {
  it("passes only the payload to the handler and returns the unlisten function", async () => {
    const unlisten = vi.fn();
    listenMock.mockImplementation((_event, callback) => {
      callback({ event: "meeting:ready", id: 1, payload: { meetingId: "m1" } });
      return Promise.resolve(unlisten);
    });
    const handler = vi.fn();

    await expect(on("meeting:ready", handler)).resolves.toBe(unlisten);
    expect(listenMock).toHaveBeenCalledWith("meeting:ready", expect.any(Function));
    expect(handler).toHaveBeenCalledWith({ meetingId: "m1" });
  });
});
