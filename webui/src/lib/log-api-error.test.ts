import { afterEach, describe, expect, it, vi } from "vitest";
import { logApiError } from "./log-api-error";

describe("API error reporting", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("logs the error and publishes its message for the app notification", () => {
    let detail: { message: string } | undefined;
    const browser = new EventTarget();
    browser.addEventListener("snapdog:api-error", (event) => {
      detail = (event as CustomEvent<{ message: string }>).detail;
    });
    vi.stubGlobal("window", browser);
    const error = new Error("GET /api/v1/zones: 500");
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => undefined);

    logApiError(error);

    expect(consoleError).toHaveBeenCalledWith("API error", error);
    expect(detail).toEqual({ message: error.message });
  });
});
