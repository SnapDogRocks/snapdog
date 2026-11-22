import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { SERVER_EQ_LIMITS } from "./eq-limits.generated";
import {
  FREQ_MAX_HZ,
  FREQ_MIN_HZ,
  GAIN_MAX_DB,
  GAIN_MIN_DB,
  MAX_EQ_BANDS,
  Q_MAX,
  Q_MIN,
} from "@/components/EqCurve";

interface Bound { minimum: number; maximum: number }

describe("EQ limits", () => {
  it("match the bounds in the server's OpenAPI document", () => {
    const doc = JSON.parse(
      readFileSync(path.resolve(__dirname, "../../../openapi.json"), "utf8"),
    ) as { components: { schemas: { EqBand: { properties: Record<string, Bound> }; EqConfig: { properties: { bands: { maxItems: number } } } } } };
    const band = doc.components.schemas.EqBand.properties;
    const range = (field: string) => ({ min: band[field]?.minimum, max: band[field]?.maximum });
    expect(SERVER_EQ_LIMITS).toEqual({
      maxBands: doc.components.schemas.EqConfig.properties.bands.maxItems,
      freqHz: range("freq"),
      gainDb: range("gain"),
      q: range("q"),
    });
  });

  it("keep the curve's editing ranges inside what the server accepts", () => {
    const { freqHz, gainDb, q, maxBands } = SERVER_EQ_LIMITS;
    expect(MAX_EQ_BANDS).toBeLessThanOrEqual(maxBands);
    expect(FREQ_MIN_HZ).toBeGreaterThanOrEqual(freqHz.min);
    expect(FREQ_MAX_HZ).toBeLessThanOrEqual(freqHz.max);
    expect(GAIN_MIN_DB).toBeGreaterThanOrEqual(gainDb.min);
    expect(GAIN_MAX_DB).toBeLessThanOrEqual(gainDb.max);
    expect(Q_MIN).toBeGreaterThanOrEqual(q.min);
    expect(Q_MAX).toBeLessThanOrEqual(q.max);
  });
});
