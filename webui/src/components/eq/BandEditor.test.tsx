import { describe, expect, it, vi } from "vitest";
import { fireEvent, screen } from "@testing-library/react";
import { useState } from "react";
import type { EqBand } from "@/lib/api";
import { renderWithIntl } from "@/test/render";
import { BandEditor } from "./BandEditor";
import { SegmentedChoice } from "./SegmentedChoice";

const band = (freq: number, gain: number): EqBand => ({ freq, gain, q: 1, type: "peaking" });

function Editor({ initial, onBands }: { initial: EqBand[]; onBands?: (bands: EqBand[]) => void }) {
  const [bands, setBands] = useState(initial);
  const set = (next: EqBand[]) => { setBands(next); onBands?.(next); };
  return (
    <BandEditor
      bands={bands}
      response={[]}
      curveLabel="EQ frequency response curve"
      onBandChange={(idx, patch) => { set(bands.map((b, i) => (i === idx ? { ...b, ...patch } : b))); }}
      onRemoveBand={(idx) => { set(bands.filter((_, i) => i !== idx)); }}
      onAddBand={(b) => { set([...bands, b]); }}
    />
  );
}

describe("BandEditor", () => {
  it("changes every value of a band with the keyboard", () => {
    const onBands = vi.fn();
    renderWithIntl(<Editor initial={[band(1000, 3)]} onBands={onBands} />);

    const gain = screen.getByRole("slider", { name: "Band 1 gain" });
    gain.focus();
    fireEvent.keyDown(gain, { key: "ArrowRight" });
    expect(onBands).toHaveBeenLastCalledWith([expect.objectContaining({ gain: 3.5 })]);

    fireEvent.keyDown(screen.getByRole("slider", { name: "Band 1 frequency" }), { key: "ArrowRight" });
    expect((onBands.mock.lastCall?.[0] as EqBand[])[0]?.freq).toBeGreaterThan(1000);

    fireEvent.keyDown(screen.getByRole("slider", { name: "Band 1 Q factor" }), { key: "ArrowRight" });
    expect(onBands).toHaveBeenLastCalledWith([expect.objectContaining({ q: 1.1 })]);

    fireEvent.change(screen.getByRole("combobox", { name: "Band 1 filter type" }), { target: { value: "low_shelf" } });
    expect(onBands).toHaveBeenLastCalledWith([expect.objectContaining({ type: "low_shelf" })]);
  });

  it("keeps focus and values on the right rows when a middle band is removed", () => {
    renderWithIntl(<Editor initial={[band(100, 1), band(1000, 2), band(5000, 3)]} />);
    const third = screen.getByRole("slider", { name: "Band 3 gain" });
    third.focus();

    fireEvent.click(screen.getByRole("button", { name: "Remove band 2" }));

    // The row of the former third band is now the second, and it still has the focus.
    const second = screen.getByRole("slider", { name: "Band 2 gain" });
    expect(document.activeElement).toBe(second);
    expect(second.getAttribute("aria-valuenow")).toBe("3");
    expect(screen.getByRole("slider", { name: "Band 1 gain" }).getAttribute("aria-valuenow")).toBe("1");
    expect(screen.queryByRole("slider", { name: "Band 3 gain" })).toBeNull();
  });
});

describe("SegmentedChoice", () => {
  function Toggle() {
    const [value, setValue] = useState<"off" | "on">("off");
    return (
      <SegmentedChoice
        label="EQ on/off"
        options={[{ value: "off", label: "Off" }, { value: "on", label: "On" }]}
        value={value}
        onChange={setValue}
      />
    );
  }

  it("has one tab stop and moves the selection with the arrow keys", () => {
    renderWithIntl(<Toggle />);
    const off = screen.getByRole("radio", { name: "Off" });
    const on = screen.getByRole("radio", { name: "On" });
    expect([off.tabIndex, on.tabIndex]).toEqual([0, -1]);

    off.focus();
    fireEvent.keyDown(off, { key: "ArrowRight" });
    expect(on.getAttribute("aria-checked")).toBe("true");
    expect(document.activeElement).toBe(on);
    expect([off.tabIndex, on.tabIndex]).toEqual([-1, 0]);

    fireEvent.keyDown(on, { key: "ArrowRight" });
    expect(off.getAttribute("aria-checked")).toBe("true");
    fireEvent.keyDown(off, { key: "End" });
    expect(on.getAttribute("aria-checked")).toBe("true");
  });
});
