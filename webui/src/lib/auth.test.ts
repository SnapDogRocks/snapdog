import { afterEach, describe, expect, it, vi } from "vitest";

describe("API key changes", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("dispatches the auth-changed event when credentials are set or cleared", async () => {
    const events: string[] = [];
    const browser = new EventTarget();
    browser.addEventListener("snapdog:auth-changed", () => events.push("changed"));
    vi.stubGlobal("window", browser);
    vi.stubGlobal("sessionStorage", { setItem: vi.fn(), removeItem: vi.fn() });
    vi.stubGlobal("localStorage", { removeItem: vi.fn() });

    const { clearApiKey, setApiKey } = await import("./auth");
    setApiKey("new-key");
    clearApiKey();

    expect(events).toHaveLength(2);
  });
});
