import { describe, expect, it } from "vitest";
import { computeResponse } from "./eq-response";
import type { EqBand } from "./types";

/** Point in the response array whose frequency is closest to `target`. */
function nearest(points: { freq: number; db: number }[], target: number) {
  return points.reduce((best, p) =>
    Math.abs(p.freq - target) < Math.abs(best.freq - target) ? p : best,
  );
}

describe("computeResponse", () => {
  it("returns the requested number of points spanning 20 Hz–20 kHz", () => {
    const points = computeResponse([], 48_000, 10);
    expect(points).toHaveLength(10);
    expect(points[0]?.freq).toBeCloseTo(20, 0);
    expect(points[9]?.freq).toBeCloseTo(20_000, -1);
  });

  it("is flat at 0 dB with no bands", () => {
    const points = computeResponse([]);
    for (const p of points) {
      expect(p.db).toBe(0);
    }
  });

  it("boosts near the center frequency of a peaking band", () => {
    const band: EqBand = { freq: 1_000, gain: 6, q: 1, type: "peaking" };
    const points = computeResponse([band]);
    const atCenter = nearest(points, 1_000);
    // The peak of a peaking filter sits close to its configured gain.
    expect(atCenter.db).toBeGreaterThan(4);
    expect(atCenter.db).toBeLessThan(7);
  });

  it("leaves frequencies far from a peaking band's center largely unaffected", () => {
    const band: EqBand = { freq: 1_000, gain: 6, q: 1, type: "peaking" };
    const points = computeResponse([band]);
    const farBelow = nearest(points, 50);
    const farAbove = nearest(points, 15_000);
    expect(Math.abs(farBelow.db)).toBeLessThan(1);
    expect(Math.abs(farAbove.db)).toBeLessThan(1);
  });

  it("cuts near the center frequency of a negative-gain band", () => {
    const band: EqBand = { freq: 1_000, gain: -6, q: 1, type: "peaking" };
    const points = computeResponse([band]);
    const atCenter = nearest(points, 1_000);
    expect(atCenter.db).toBeLessThan(-4);
    expect(atCenter.db).toBeGreaterThan(-7);
  });

  it("sums multiple bands", () => {
    const bands: EqBand[] = [
      { freq: 1_000, gain: 3, q: 1, type: "peaking" },
      { freq: 1_000, gain: 3, q: 1, type: "peaking" },
    ];
    const single = computeResponse([bands[0]!]);
    const double = computeResponse(bands);
    const atCenterSingle = nearest(single, 1_000).db;
    const atCenterDouble = nearest(double, 1_000).db;
    // Two identical bands should boost roughly twice as much as one.
    expect(atCenterDouble).toBeGreaterThan(atCenterSingle * 1.5);
  });
});
