"use client";

import { useState } from "react";
import { useTranslations } from "next-intl";
import { Button } from "@/components/ui/button";
import { Slider } from "@/components/ui/slider";
import type { EqBand } from "@/lib/api";
import {
  FILTER_TYPES,
  MAX_EQ_BANDS,
  FREQ_MIN_HZ,
  FREQ_MAX_HZ,
  GAIN_MIN_DB,
  GAIN_MAX_DB,
  GAIN_STEP_DB,
  Q_MIN,
  Q_MAX,
  Q_STEP,
  InteractiveEQCurve,
} from "@/components/EqCurve";

export const DEFAULT_BAND: EqBand = { freq: 1000, gain: 0, q: 1.0, type: "peaking" };

let nextBandKey = 0;
const newBandKey = () => `band-${++nextBandKey}`;

/**
 * Stable keys for a band list. Removing a band removes its key, so the rows after it keep
 * their focus and slider state; a list replaced from outside (preset, server) gets new keys.
 */
function useBandKeys(count: number) {
  const [keys, setKeys] = useState<string[]>(() => Array.from({ length: count }, newBandKey));
  let current = keys;
  if (keys.length !== count) {
    current = Array.from({ length: count }, newBandKey);
    setKeys(current);
  }
  return {
    keys: current,
    remove: (idx: number) => { setKeys(current.filter((_, i) => i !== idx)); },
    add: () => { setKeys([...current, newBandKey()]); },
  };
}

/**
 * The curve with its band nodes and one editable row per band: the same editor for the
 * zone or client EQ and the custom speaker correction.
 */
export function BandEditor({
  bands,
  response,
  curveLabel,
  onBandChange,
  onRemoveBand,
  onAddBand,
}: {
  bands: EqBand[];
  response: { freq: number; db: number }[];
  curveLabel: string;
  onBandChange: (idx: number, patch: Partial<EqBand>) => void;
  onRemoveBand: (idx: number) => void;
  onAddBand: (band: EqBand) => void;
}) {
  const t = useTranslations("eq");
  const [selected, setSelected] = useState<number | null>(null);
  const bandKeys = useBandKeys(bands.length);

  const remove = (idx: number) => {
    bandKeys.remove(idx);
    if (selected === idx) setSelected(null);
    else if (selected !== null && selected > idx) setSelected(selected - 1);
    onRemoveBand(idx);
  };
  const add = (band: EqBand, select: boolean) => {
    if (bands.length >= MAX_EQ_BANDS) return;
    bandKeys.add();
    if (select) setSelected(bands.length);
    onAddBand(band);
  };

  return (
    <>
      <InteractiveEQCurve
        bands={bands}
        bandKeys={bandKeys.keys}
        response={response}
        curveLabel={curveLabel}
        selectedBand={selected}
        onSelectBand={setSelected}
        onBandChange={onBandChange}
        onRemoveBand={remove}
        onAddBand={(freq, gain) => {
          add({ ...DEFAULT_BAND, freq: Math.round(freq), gain: Math.round(gain * 2) / 2 }, true);
        }}
      />
      <div className="space-y-3">
        {bands.map((band, idx) => (
          <BandRow
            key={bandKeys.keys[idx] ?? idx}
            band={band}
            index={idx}
            onChange={(patch) => { onBandChange(idx, patch); }}
            onRemove={() => { remove(idx); }}
          />
        ))}
      </div>
      {bands.length < MAX_EQ_BANDS && (
        <Button variant="ghost" size="sm" onClick={() => { add({ ...DEFAULT_BAND }, false); }} className="w-full">
          {t("addBand")}
        </Button>
      )}
    </>
  );
}

function BandRow({
  band,
  index,
  onChange,
  onRemove,
}: {
  band: EqBand;
  index: number;
  onChange: (patch: Partial<EqBand>) => void;
  onRemove: () => void;
}) {
  const t = useTranslations("eq");
  return (
    <div className="flex items-center gap-2 text-sm">
      <span className="w-5 text-muted-foreground text-xs">{index + 1}</span>
      <select
        value={band.type}
        onChange={(e) => { onChange({ type: e.target.value as EqBand["type"] }); }}
        className="bg-muted border border-border rounded px-1.5 py-1 text-xs w-20"
        aria-label={t("filterType", { n: index + 1 })}
      >
        {FILTER_TYPES.map((filter) => (
          <option key={filter} value={filter}>{t(`filter.${filter}`)}</option>
        ))}
      </select>
      <div className="flex-1 space-y-0.5">
        <div className="flex items-center gap-2">
          <span className="w-8 text-xs text-muted-foreground" aria-hidden="true">Hz</span>
          <Slider
            value={[Math.log10(band.freq)]}
            min={Math.log10(FREQ_MIN_HZ)}
            max={Math.log10(FREQ_MAX_HZ)}
            step={0.01}
            onValueChange={([v]) => { if (v !== undefined) onChange({ freq: Math.round(Math.pow(10, v)) }); }}
            getAriaValueText={(value) => {
              const frequency = Math.round(Math.pow(10, value));
              return frequency >= 1000 ? `${(frequency / 1000).toFixed(1)} kHz` : `${frequency} Hz`;
            }}
            className="flex-1"
            aria-label={t("bandFrequency", { n: index + 1 })}
          />
          <span className="w-12 text-xs tabular-nums text-right">{band.freq >= 1000 ? `${(band.freq / 1000).toFixed(1)}k` : band.freq}</span>
        </div>
        <div className="flex items-center gap-2">
          <span className="w-8 text-xs text-muted-foreground" aria-hidden="true">dB</span>
          <Slider
            value={[band.gain]}
            min={GAIN_MIN_DB}
            max={GAIN_MAX_DB}
            step={GAIN_STEP_DB}
            onValueChange={([v]) => { if (v !== undefined) onChange({ gain: v }); }}
            getAriaValueText={(value) => `${value.toFixed(1)} dB`}
            className="flex-1"
            aria-label={t("bandGain", { n: index + 1 })}
          />
          <span className="w-12 text-xs tabular-nums text-right">{band.gain > 0 ? "+" : ""}{band.gain.toFixed(1)}</span>
        </div>
        <div className="flex items-center gap-2">
          <span className="w-8 text-xs text-muted-foreground" aria-hidden="true">Q</span>
          <Slider
            value={[band.q]}
            min={Q_MIN}
            max={Q_MAX}
            step={Q_STEP}
            onValueChange={([v]) => { if (v !== undefined) onChange({ q: Math.round(v * 10) / 10 }); }}
            getAriaValueText={(value) => value.toFixed(1)}
            className="flex-1"
            aria-label={t("bandQ", { n: index + 1 })}
          />
          <span className="w-12 text-xs tabular-nums text-right">{band.q.toFixed(1)}</span>
        </div>
      </div>
      <Button variant="ghost" size="sm" onClick={onRemove} className="text-muted-foreground px-1" aria-label={t("removeBandN", { n: index + 1 })}>✕</Button>
    </div>
  );
}
