import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import type { EqBand, EqConfig } from "@/lib/api";
import { EqOverlay } from "./EqOverlay";
import { renderWithIntl } from "@/test/render";
import { resetEqPresetsCache } from "@/hooks/useEqPresets";

const BAND: EqBand = { freq: 1000, gain: 3, q: 1, type: "peaking" };

const mocks = vi.hoisted(() => ({
  eqGet: vi.fn(),
  eqSet: vi.fn(),
  clientEqGet: vi.fn(),
  speakersList: vi.fn(),
  speakersGet: vi.fn(),
  applyCustom: vi.fn(),
  apply: vi.fn(),
  presets: vi.fn(),
}));

vi.mock("@/lib/api", () => ({
  api: {
    eq: { get: mocks.eqGet, set: mocks.eqSet, applyPreset: vi.fn(), presets: mocks.presets },
    clientEq: { get: mocks.clientEqGet, set: vi.fn(() => Promise.resolve()), applyPreset: vi.fn() },
    speakers: {
      list: mocks.speakersList,
      get: mocks.speakersGet,
      applyCustom: mocks.applyCustom,
      apply: mocks.apply,
    },
  },
}));

function lastCall<T>(fn: { mock: { calls: T[][] } }): T[] {
  const call = fn.mock.calls.at(-1);
  if (!call) throw new Error("not called");
  return call;
}

function abButton() {
  return screen.getByRole("button", { name: "A/B" });
}

beforeEach(() => {
  vi.clearAllMocks();
  resetEqPresetsCache();
  mocks.presets.mockResolvedValue([{ name: "flat", bands: [] }]);
  mocks.eqSet.mockResolvedValue(undefined);
  mocks.applyCustom.mockImplementation((_id: number, c: EqConfig) => Promise.resolve(c));
  mocks.speakersList.mockResolvedValue([]);
  mocks.clientEqGet.mockResolvedValue({ enabled: false, bands: [], preset: "flat" });
});

describe("EqOverlay, presets", () => {
  it("offers the server's presets, a preset without a label under its raw name", async () => {
    mocks.presets.mockResolvedValue([
      { name: "flat", bands: [] },
      { name: "bass_boost", bands: [] },
      { name: "studio_monitor", bands: [] },
    ]);
    mocks.eqGet.mockResolvedValue({ enabled: true, bands: [BAND], preset: null });
    renderWithIntl(<EqOverlay zoneId={1} label="Kitchen" onClose={vi.fn()} />);

    const group = await screen.findByRole("radiogroup", { name: "EQ presets" });
    await waitFor(() => { expect(group.querySelectorAll("[role=radio]")).toHaveLength(3); });
    expect(screen.getByRole("radio", { name: "Bass Boost" })).toBeTruthy();
    expect(screen.getByRole("radio", { name: "studio_monitor" })).toBeTruthy();
  });
});

describe("EqOverlay, EQ tab", () => {
  it("keeps the editor and A/B usable through a bypass and ends with the EQ on", async () => {
    mocks.eqGet.mockResolvedValue({ enabled: true, bands: [BAND], preset: null });
    renderWithIntl(<EqOverlay zoneId={1} label="Kitchen" onClose={vi.fn()} />);
    const addBand = await screen.findByRole("button", { name: "+ Add Band" });

    fireEvent.click(abButton());
    expect(lastCall(mocks.eqSet)[1]).toMatchObject({ enabled: false });
    expect((abButton() as HTMLButtonElement).disabled).toBe(false);
    expect(abButton().getAttribute("aria-pressed")).toBe("true");

    fireEvent.click(abButton());
    expect(lastCall(mocks.eqSet)[1]).toMatchObject({ enabled: true, bands: [BAND] });
    expect((abButton() as HTMLButtonElement).disabled).toBe(false);
    expect(addBand.closest(".pointer-events-none")).toBeNull();
  });
});

describe("EqOverlay, speaker tab", () => {
  it("sends the custom bands, switched on, after A/B and close", async () => {
    mocks.speakersGet.mockResolvedValue({ enabled: false, bands: [], preset: null });
    const { unmount } = renderWithIntl(<EqOverlay clientId={2} label="Desk" onClose={vi.fn()} />);
    fireEvent.click(await screen.findByRole("tab", { name: "Speaker" }));
    fireEvent.click(await screen.findByRole("radio", { name: "Custom" }));
    fireEvent.click(await screen.findByRole("button", { name: "+ + Add Band" }));
    expect(lastCall(mocks.applyCustom)[1]).toMatchObject({ enabled: true });

    fireEvent.click(abButton());
    expect(lastCall(mocks.applyCustom)[1]).toMatchObject({ enabled: false });
    fireEvent.click(abButton());
    fireEvent.click(abButton());
    fireEvent.click(screen.getByRole("button", { name: "Close equalizer" }));
    act(() => { unmount(); });

    const last = lastCall(mocks.applyCustom)[1] as EqConfig;
    expect(last.enabled).toBe(true);
    expect(last.bands).toHaveLength(1);
  });

  it("shows a correction loaded switched off with bands as bypassed", async () => {
    mocks.speakersGet.mockResolvedValue({ enabled: false, bands: [BAND], preset: null });
    renderWithIntl(<EqOverlay clientId={2} label="Desk" onClose={vi.fn()} />);
    fireEvent.click(await screen.findByRole("tab", { name: "Speaker" }));

    await waitFor(() => { expect(abButton().getAttribute("aria-pressed")).toBe("true"); });
    expect(screen.getByRole("radio", { name: "Custom" }).getAttribute("aria-checked")).toBe("true");
    expect((abButton() as HTMLButtonElement).disabled).toBe(false);
  });
});
