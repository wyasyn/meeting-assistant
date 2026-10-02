import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import App from "@/App";
import { APP_NAME } from "@/config";

describe("App", () => {
  it("shows the app name", () => {
    render(<App />);
    expect(screen.getByRole("heading", { name: APP_NAME })).toBeInTheDocument();
  });

  // FR-1.3: nothing can start a recording until consent and capture exist.
  it("keeps the record button disabled", () => {
    render(<App />);
    expect(screen.getByRole("button", { name: /recording/i })).toBeDisabled();
  });
});
