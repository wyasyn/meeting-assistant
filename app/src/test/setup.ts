import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

// jsdom does not lay out, so it has no scrolling.
Element.prototype.scrollIntoView = () => undefined;

afterEach(() => {
  cleanup();
});
