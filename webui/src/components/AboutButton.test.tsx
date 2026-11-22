import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, screen, waitFor } from "@testing-library/react";
import { AboutButton } from "./AboutButton";
import { renderWithIntl } from "@/test/render";

const mocks = vi.hoisted(() => ({
  setProgrammingMode: vi.fn(),
  downloadKnxprod: vi.fn(),
  checkUpdates: vi.fn(),
}));

vi.mock("@/lib/api", () => ({
  api: {
    system: {
      version: vi.fn(() => Promise.resolve({ version: "1.0.0", name: "SnapDog", rust_version: "" })),
      updateStatus: vi.fn(() => Promise.resolve({
        state: "idle",
        currentVersion: "1.0.0",
        availableVersion: null,
        updateAvailable: false,
        lastError: null,
        updateMode: "manual",
        maintenanceTime: "03:00",
        timezone: "UTC",
        progress: null,
      })),
      checkUpdates: mocks.checkUpdates,
      applyUpdate: vi.fn(),
    },
    knx: {
      getProgrammingMode: vi.fn(() => Promise.resolve(false)),
      getProductInfo: vi.fn(() => Promise.reject(new Error("no KNX"))),
      setProgrammingMode: mocks.setProgrammingMode,
      downloadKnxprod: mocks.downloadKnxprod,
    },
  },
}));

const REFUSED = new Error("Programming mode is switched off on this server");

describe("AboutButton, failed operations", () => {
  let toasts: string[];
  const onToast = (e: Event) => { toasts.push((e as CustomEvent<{ message: string }>).detail.message); };

  beforeEach(() => {
    toasts = [];
    window.addEventListener("snapdog:api-error", onToast);
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    mocks.setProgrammingMode.mockRejectedValue(REFUSED);
    mocks.downloadKnxprod.mockRejectedValue(new Error("download refused"));
    mocks.checkUpdates.mockRejectedValue(new Error("update check refused"));
  });

  afterEach(() => {
    window.removeEventListener("snapdog:api-error", onToast);
    vi.restoreAllMocks();
  });

  function open() {
    renderWithIntl(<AboutButton />);
    fireEvent.click(screen.getByRole("button", { name: "About" }));
  }

  it("shows the server's reason when the programming mode switch is refused", async () => {
    open();
    fireEvent.click(await screen.findByRole("switch"));
    await waitFor(() => { expect(toasts).toEqual([REFUSED.message]); });
    expect(screen.getByRole("switch").getAttribute("aria-checked")).toBe("false");
  });

  it("shows a failed knxprod download and a failed update check", async () => {
    open();
    await screen.findByRole("switch");
    const download = screen.getAllByRole("button").find((b) => /knxprod|ETS/i.test(b.textContent));
    if (!download) throw new Error("download button not rendered");
    fireEvent.click(download);
    await waitFor(() => { expect(toasts).toContain("download refused"); });

    const check = await screen.findByRole("button", { name: /check/i });
    fireEvent.click(check);
    await waitFor(() => { expect(toasts).toContain("update check refused"); });
  });
});
