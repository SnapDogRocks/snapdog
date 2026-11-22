import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import type { EqBand, EqConfig } from "@/lib/api";
import { EqOverlay } from "./EqOverlay";
import { renderWithIntl } from "@/test/render";
import { resetEqPresetsCache } from "@/hooks/useEqPresets";
import { publishEqChange } from "@/lib/eq-events";
import { FREQ_MAX_HZ, FREQ_MIN_HZ } from "./EqCurve";

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
    await waitFor(() => { expect(lastCall(mocks.eqSet)[1]).toMatchObject({ enabled: false }); });
    expect((abButton() as HTMLButtonElement).disabled).toBe(false);
    expect(abButton().getAttribute("aria-pressed")).toBe("true");

    fireEvent.click(abButton());
    await waitFor(() => { expect(lastCall(mocks.eqSet)[1]).toMatchObject({ enabled: true, bands: [BAND] }); });
    expect((abButton() as HTMLButtonElement).disabled).toBe(false);
    expect(addBand.closest(".pointer-events-none")).toBeNull();
  });
});

describe("EqOverlay, sending edits", () => {
  /** The curve maps 1 px to 1 viewBox unit; the plot starts at x 35 and is 555 wide. */
  function curve() {
    const svg = document.querySelector("svg.cursor-crosshair");
    if (!svg) throw new Error("curve not rendered");
    svg.getBoundingClientRect = () => new DOMRect(0, 0, 600, 160);
    const node = svg.querySelector("circle");
    if (!node) throw new Error("band node not rendered");
    return { svg, node };
  }

  function freqAt(x: number) {
    const lo = Math.log10(FREQ_MIN_HZ);
    const hi = Math.log10(FREQ_MAX_HZ);
    return Math.round(Math.pow(10, lo + ((x - 35) / 555) * (hi - lo)));
  }

  it("bounds the requests of a drag and sends the final band last", async () => {
    mocks.eqGet.mockResolvedValue({ enabled: true, bands: [BAND], preset: null });
    let release: () => void = () => undefined;
    mocks.eqSet.mockImplementationOnce(() => new Promise<void>((resolve) => { release = resolve; }));
    renderWithIntl(<EqOverlay zoneId={1} label="Kitchen" onClose={vi.fn()} />);
    await screen.findByRole("button", { name: "+ Add Band" });
    const { svg, node } = curve();

    fireEvent.pointerDown(node, { button: 0, pointerId: 1 });
    for (let i = 0; i < 100; i++) {
      fireEvent.pointerMove(svg, { pointerId: 1, clientX: 100 + i * 2, clientY: 50 });
    }
    // y 50 is +6 dB on this curve.
    const final = { freq: freqAt(298), gain: 6 };
    act(() => { release(); });
    await waitFor(() => { expect(lastCall(mocks.eqSet)[1]).toMatchObject({ bands: [final] }); });
    expect(mocks.eqSet.mock.calls.length).toBeLessThanOrEqual(2);

    // The same rounded values again (y 49.9 still rounds to +6 dB): nothing is sent.
    const sent = mocks.eqSet.mock.calls.length;
    fireEvent.pointerMove(svg, { pointerId: 1, clientX: 298, clientY: 50 });
    fireEvent.pointerMove(svg, { pointerId: 1, clientX: 298, clientY: 49.9 });
    await act(async () => { await Promise.resolve(); });
    expect(mocks.eqSet.mock.calls.length).toBe(sent);
  });

  it("restores the server config after a failed request", async () => {
    mocks.eqGet
      .mockResolvedValueOnce({ enabled: true, bands: [BAND], preset: null })
      .mockResolvedValueOnce({ enabled: true, bands: [{ ...BAND, gain: -2 }], preset: null });
    mocks.eqSet.mockRejectedValueOnce(new Error("422"));
    renderWithIntl(<EqOverlay zoneId={1} label="Kitchen" onClose={vi.fn()} />);
    await screen.findByText("+3.0");

    fireEvent.click(screen.getByRole("button", { name: "Remove band" }));
    expect(screen.queryByText("+3.0")).toBeNull();
    await screen.findByText("-2.0");
    expect(mocks.eqGet).toHaveBeenCalledTimes(2);
  });

  it("applies an EQ change from another controller", async () => {
    mocks.eqGet.mockResolvedValue({ enabled: true, bands: [BAND], preset: null });
    renderWithIntl(<EqOverlay zoneId={1} label="Kitchen" onClose={vi.fn()} />);
    await screen.findByText("+3.0");

    act(() => { publishEqChange("zone:1", { enabled: true, bands: [{ ...BAND, gain: -4 }], preset: null }); });
    expect(screen.getByText("-4.0")).toBeTruthy();
    act(() => { publishEqChange("zone:2", { enabled: true, bands: [{ ...BAND, gain: 9 }], preset: null }); });
    expect(screen.queryByText("+9.0")).toBeNull();
  });
});

describe("EqOverlay, speaker tab", () => {
  it("sends the custom bands, switched on, after A/B and close", async () => {
    mocks.speakersGet.mockResolvedValue({ enabled: false, bands: [], preset: null });
    const { unmount } = renderWithIntl(<EqOverlay clientId={2} label="Desk" onClose={vi.fn()} />);
    fireEvent.click(await screen.findByRole("tab", { name: "Speaker" }));
    fireEvent.click(await screen.findByRole("radio", { name: "Custom" }));
    fireEvent.click(await screen.findByRole("button", { name: "+ + Add Band" }));
    await waitFor(() => { expect(lastCall(mocks.applyCustom)[1]).toMatchObject({ enabled: true }); });

    fireEvent.click(abButton());
    await waitFor(() => { expect(lastCall(mocks.applyCustom)[1]).toMatchObject({ enabled: false }); });
    fireEvent.click(abButton());
    fireEvent.click(abButton());
    fireEvent.click(screen.getByRole("button", { name: "Close equalizer" }));
    act(() => { unmount(); });

    await waitFor(() => {
      const last = lastCall(mocks.applyCustom)[1] as EqConfig;
      expect(last.enabled).toBe(true);
      expect(last.bands).toHaveLength(1);
    });
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
