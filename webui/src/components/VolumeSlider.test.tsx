import { describe, expect, it, vi } from "vitest";
import { act, fireEvent, screen } from "@testing-library/react";
import { VolumeSlider } from "./VolumeSlider";
import { layoutSlider, renderWithIntl } from "@/test/render";

function slider(props: { volume: number; max?: number; onVolumeChange?: (v: number) => void }) {
  return (
    <VolumeSlider
      volume={props.volume}
      muted={false}
      max={props.max ?? 100}
      onVolumeChange={props.onVolumeChange ?? vi.fn()}
      onMuteToggle={vi.fn()}
      onUnmute={vi.fn()}
    />
  );
}

function sliderRoot() {
  const root = document.querySelector('[data-slot="slider"]');
  if (!root) throw new Error("slider not rendered");
  layoutSlider(root);
  return root;
}

function shownVolume() {
  return screen.getByRole("slider").getAttribute("aria-valuenow");
}

async function release(root: Element, clientX: number) {
  fireEvent.pointerUp(root, { button: 0, pointerId: 1, clientX });
  // The component commits a release without a Radix commit on the next tick.
  await act(async () => { await new Promise((r) => setTimeout(r, 0)); });
}

describe("VolumeSlider", () => {
  it("follows the server after a release that Radix does not commit", async () => {
    const onVolumeChange = vi.fn();
    const { rerender } = renderWithIntl(slider({ volume: 50, onVolumeChange }));
    const root = sliderRoot();

    // Drag away and back to the start: Radix sees no change and does not commit.
    fireEvent.pointerDown(root, { button: 0, pointerId: 1, clientX: 70 });
    fireEvent.pointerMove(root, { button: 0, pointerId: 1, clientX: 50 });
    await release(root, 50);
    expect(shownVolume()).toBe("50");

    rerender(slider({ volume: 40, onVolumeChange }));
    expect(shownVolume()).toBe("40");
  });

  it("follows the server after a click beyond the max_volume cap", async () => {
    const { rerender } = renderWithIntl(slider({ volume: 60, max: 60 }));
    const root = sliderRoot();

    fireEvent.pointerDown(root, { button: 0, pointerId: 1, clientX: 80 });
    await release(root, 80);
    expect(shownVolume()).toBe("60");

    rerender(slider({ volume: 40, max: 60 }));
    expect(shownVolume()).toBe("40");
  });
});
