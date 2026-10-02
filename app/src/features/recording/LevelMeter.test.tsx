import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { LevelMeter } from "./LevelMeter";

describe("LevelMeter (FR-1.5)", () => {
  it("maps dBFS onto the bar", () => {
    render(<LevelMeter label="You" db={-45} />);
    expect(screen.getByRole("meter", { name: "You" })).toHaveAttribute("aria-valuenow", "50");
    expect(screen.queryByText("No audio from this device")).not.toBeInTheDocument();
  });

  it("says so when a device sends nothing", () => {
    render(<LevelMeter label="Others" db={null} />);
    expect(screen.getByText("No audio from this device")).toBeInTheDocument();
    expect(screen.getByRole("meter", { name: "Others" })).toHaveAttribute("aria-valuenow", "0");
  });
});
