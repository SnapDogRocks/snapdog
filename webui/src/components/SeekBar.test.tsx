import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, screen } from "@testing-library/react";
import { SeekBar } from "./SeekBar";
import type { ZoneState } from "@/stores/useAppStore";
import { layoutSlider, renderWithIntl } from "@/test/render";

vi.mock("@/lib/api", () => ({ api: { zones: { seekPosition: vi.fn(() => Promise.resolve()) } } }));

const DURATION_MS = 100_000;

function playingZone(positionMs: number): ZoneState {
  return {
    index: 1,
    playback: "playing",
    source: "subsonic",
    buffered_ms: null,
    track: { title: "T", artist: "A", duration_ms: DURATION_MS, position_ms: positionMs, seekable: true },
  } as unknown as ZoneState;
}

function position() {
  return Number(screen.getByRole("slider").getAttribute("aria-valuenow"));
}

describe("SeekBar", () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  it("resumes interpolation after a drag released at its start position", () => {
    renderWithIntl(<SeekBar zone={playingZone(10_000)} />);
    const root = document.querySelector('[data-slot="slider"]');
    if (!root) throw new Error("slider not rendered");
    layoutSlider(root); // 1 px = 1 s

    fireEvent.pointerDown(root, { button: 0, pointerId: 1, clientX: 10 });
    fireEvent.pointerMove(root, { button: 0, pointerId: 1, clientX: 30 });
    expect(position()).toBe(30_000);
    fireEvent.pointerMove(root, { button: 0, pointerId: 1, clientX: 10 });
    fireEvent.pointerUp(root, { button: 0, pointerId: 1, clientX: 10 });

    act(() => { vi.advanceTimersByTime(0); }); // the release is handled on the next tick
    act(() => { vi.advanceTimersByTime(1_000); });
    expect(position()).toBe(11_000);
  });
});
