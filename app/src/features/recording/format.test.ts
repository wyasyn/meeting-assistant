import { describe, expect, it } from "vitest";
import { defaultTitle, formatElapsed } from "./format";

describe("formatElapsed (FR-1.5)", () => {
  it.each([
    [0, "0:00"],
    [999, "0:00"],
    [61_000, "1:01"],
    [3_599_000, "59:59"],
    [3_600_000, "1:00:00"],
    [3_725_000, "1:02:05"],
    [-5, "0:00"],
  ])("%d ms is %s", (ms, text) => {
    expect(formatElapsed(ms)).toBe(text);
  });
});

describe("defaultTitle", () => {
  it("names the meeting after the local time", () => {
    expect(defaultTitle(new Date(2026, 9, 2, 14, 5))).toMatch(/^Meeting on .*2026/);
  });
});
