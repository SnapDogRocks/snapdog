"use client";

import { useRef, useState } from "react";
import type { EqBand } from "@/lib/api";

export const FILTER_TYPES: readonly EqBand["type"][] = ["low_shelf", "high_shelf", "peaking", "low_pass", "high_pass"] as const;
export const MAX_EQ_BANDS = 10;
export const FREQ_MIN_HZ = 20;
export const FREQ_MAX_HZ = 20000;
export const GAIN_MIN_DB = -12;
export const GAIN_MAX_DB = 12;
export const GAIN_STEP_DB = 0.5;
export const Q_MIN = 0.1;
export const Q_MAX = 10;
export const Q_STEP = 0.1;
export const CURVE_WIDTH = 600;
export const CURVE_HEIGHT = 160;
export const CURVE_DB_RANGE = 15;
export const CURVE_MIN_HEIGHT = 120;
export const CURVE_COLOR = "oklch(0.65 0.18 40)";
export const GRID_FREQS = [20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000] as const;
export const GRID_DBS = [-12, -6, 0, 6, 12] as const;

export function InteractiveEQCurve({
  bands,
  response,
  selectedBand,
  onSelectBand,
  onBandChange,
  onAddBand,
  onRemoveBand,
}: {
  bands: EqBand[];
  response: { freq: number; db: number }[];
  selectedBand: number | null;
  onSelectBand: (idx: number | null) => void;
  onBandChange: (idx: number, patch: Partial<EqBand>) => void;
  onAddBand: (freq: number, gain: number) => void;
  onRemoveBand: (idx: number) => void;
}) {
  const svgRef = useRef<SVGSVGElement>(null);
  const [dragging, setDragging] = useState<number | null>(null);
  const [dragOutOfBounds, setDragOutOfBounds] = useState(false);

  const width = CURVE_WIDTH;
  const height = CURVE_HEIGHT;
  const pad = { top: 10, right: 10, bottom: 20, left: 35 };
  const plotW = width - pad.left - pad.right;
  const plotH = height - pad.top - pad.bottom;
  const dbRange = CURVE_DB_RANGE;

  const freqToX = (f: number) => pad.left + ((Math.log10(f) - Math.log10(FREQ_MIN_HZ)) / (Math.log10(FREQ_MAX_HZ) - Math.log10(FREQ_MIN_HZ))) * plotW;
  const xToFreq = (x: number) => Math.pow(10, Math.log10(FREQ_MIN_HZ) + ((x - pad.left) / plotW) * (Math.log10(FREQ_MAX_HZ) - Math.log10(FREQ_MIN_HZ)));
  const dbToY = (db: number) => pad.top + plotH / 2 - (db / dbRange) * (plotH / 2);
  const yToDb = (y: number) => -(y - pad.top - plotH / 2) / (plotH / 2) * dbRange;

  const svgPoint = (e: React.PointerEvent) => {
    const svg = svgRef.current;
    if (!svg) return { x: 0, y: 0 };
    const rect = svg.getBoundingClientRect();
    const scaleX = width / rect.width;
    const scaleY = height / rect.height;
    return { x: (e.clientX - rect.left) * scaleX, y: (e.clientY - rect.top) * scaleY };
  };

  const handlePointerMove = (e: React.PointerEvent) => {
    if (dragging === null) return;
    e.preventDefault();
    const pt = svgPoint(e);
    const outOfBounds = pt.y < pad.top - 15 || pt.y > pad.top + plotH + 15;
    setDragOutOfBounds(outOfBounds);
    if (!outOfBounds) {
      const freq = Math.max(FREQ_MIN_HZ, Math.min(FREQ_MAX_HZ, xToFreq(pt.x)));
      const gain = Math.max(GAIN_MIN_DB, Math.min(GAIN_MAX_DB, yToDb(pt.y)));
      onBandChange(dragging, { freq: Math.round(freq), gain: Math.round(gain * 2) / 2 });
    }
  };

  const handlePointerUp = () => {
    if (dragging !== null && dragOutOfBounds) {
      onRemoveBand(dragging);
      onSelectBand(null);
    }
    setDragging(null);
    setDragOutOfBounds(false);
  };

  const handleDoubleClick = (e: React.MouseEvent) => {
    const svg = svgRef.current;
    if (!svg) return;
    const rect = svg.getBoundingClientRect();
    const scaleX = width / rect.width;
    const scaleY = height / rect.height;
    const x = (e.clientX - rect.left) * scaleX;
    const y = (e.clientY - rect.top) * scaleY;
    if (x < pad.left || x > pad.left + plotW || y < pad.top || y > pad.top + plotH) return;
    onAddBand(xToFreq(x), yToDb(y));
  };

  const path = response.length > 0
    ? response.map((p, i) => `${i === 0 ? "M" : "L"}${freqToX(p.freq).toFixed(1)},${dbToY(p.db).toFixed(1)}`).join("")
    : "";

  return (
    <svg
      ref={svgRef}
      viewBox={`0 0 ${width} ${height}`}
      className="w-full rounded-lg bg-muted/30 border border-border cursor-crosshair select-none touch-none"
      style={{ minHeight: CURVE_MIN_HEIGHT }}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
      onPointerLeave={handlePointerUp}
      onDoubleClick={handleDoubleClick}
    >
      {/* Grid */}
      {GRID_FREQS.map((f) => (
        <line key={`f${f}`} x1={freqToX(f)} x2={freqToX(f)} y1={pad.top} y2={pad.top + plotH} stroke="currentColor" strokeOpacity={0.1} />
      ))}
      {GRID_DBS.map((db) => (
        <g key={`db${db}`}>
          <line x1={pad.left} x2={pad.left + plotW} y1={dbToY(db)} y2={dbToY(db)} stroke="currentColor" strokeOpacity={db === 0 ? 0.3 : 0.1} />
          <text x={pad.left - 4} y={dbToY(db) + 3} textAnchor="end" fontSize={9} fill="currentColor" opacity={0.4}>{db}</text>
        </g>
      ))}
      {[100, 1000, 10000].map((f) => (
        <text key={`fl${f}`} x={freqToX(f)} y={height - 4} textAnchor="middle" fontSize={9} fill="currentColor" opacity={0.4}>
          {f >= 1000 ? `${f / 1000}k` : f}
        </text>
      ))}
      {/* Curve fill + stroke */}
      {path && (
        <>
          <path d={path + `L${freqToX(FREQ_MAX_HZ)},${dbToY(0)}L${freqToX(FREQ_MIN_HZ)},${dbToY(0)}Z`} fill={CURVE_COLOR} fillOpacity={0.15} />
          <path d={path} fill="none" stroke={CURVE_COLOR} strokeWidth={2} />
        </>
      )}
      {/* Draggable band nodes */}
      {bands.map((band, idx) => (
        <circle
          key={idx}
          cx={freqToX(band.freq)}
          cy={dbToY(band.gain)}
          r={selectedBand === idx ? 7 : 5}
          fill={selectedBand === idx ? CURVE_COLOR : "white"}
          stroke={CURVE_COLOR}
          strokeWidth={2}
          opacity={dragging === idx && dragOutOfBounds ? 0.3 : 1}
          className="cursor-grab active:cursor-grabbing"
          onPointerDown={(e) => {
            e.preventDefault();
            e.stopPropagation();
            setDragging(idx);
            onSelectBand(idx);
            (e.target as SVGElement).setPointerCapture(e.pointerId);
          }}
          onClick={(e) => {
            e.stopPropagation();
            onSelectBand(selectedBand === idx ? null : idx);
          }}
        />
      ))}
      <text x={pad.left - 4} y={dbToY(0) + 3} textAnchor="end" fontSize={9} fill="currentColor" opacity={0.6} fontWeight="bold">0</text>
    </svg>
  );
}

export function FrequencyResponseCurve({ response, curveLabel }: { response: { freq: number; db: number }[]; curveLabel: string }) {
  const width = CURVE_WIDTH;
  const height = CURVE_HEIGHT;
  const pad = { top: 10, right: 10, bottom: 20, left: 35 };
  const plotW = width - pad.left - pad.right;
  const plotH = height - pad.top - pad.bottom;
  const dbRange = CURVE_DB_RANGE;

  const freqToX = (f: number) => pad.left + ((Math.log10(f) - Math.log10(FREQ_MIN_HZ)) / (Math.log10(FREQ_MAX_HZ) - Math.log10(FREQ_MIN_HZ))) * plotW;
  const dbToY = (db: number) => pad.top + plotH / 2 - (db / dbRange) * (plotH / 2);

  const path = response.length > 0
    ? response.map((p, i) => `${i === 0 ? "M" : "L"}${freqToX(p.freq).toFixed(1)},${dbToY(p.db).toFixed(1)}`).join("")
    : "";

  return (
    <svg viewBox={`0 0 ${width} ${height}`} className="w-full rounded-lg bg-muted/30 border border-border" style={{ minHeight: CURVE_MIN_HEIGHT }} role="img" aria-label={curveLabel}>
      {GRID_FREQS.map((f) => (
        <line key={`f${f}`} x1={freqToX(f)} x2={freqToX(f)} y1={pad.top} y2={pad.top + plotH} stroke="currentColor" strokeOpacity={0.1} />
      ))}
      {GRID_DBS.map((db) => (
        <g key={`db${db}`}>
          <line x1={pad.left} x2={pad.left + plotW} y1={dbToY(db)} y2={dbToY(db)} stroke="currentColor" strokeOpacity={db === 0 ? 0.3 : 0.1} />
          <text x={pad.left - 4} y={dbToY(db) + 3} textAnchor="end" fontSize={9} fill="currentColor" opacity={0.4}>{db}</text>
        </g>
      ))}
      {[100, 1000, 10000].map((f) => (
        <text key={`fl${f}`} x={freqToX(f)} y={height - 4} textAnchor="middle" fontSize={9} fill="currentColor" opacity={0.4}>
          {f >= 1000 ? `${f / 1000}k` : f}
        </text>
      ))}
      {path && (
        <>
          <path d={path + `L${freqToX(FREQ_MAX_HZ)},${dbToY(0)}L${freqToX(FREQ_MIN_HZ)},${dbToY(0)}Z`} fill={CURVE_COLOR} fillOpacity={0.15} />
          <path d={path} fill="none" stroke={CURVE_COLOR} strokeWidth={2} />
        </>
      )}
      <text x={pad.left - 4} y={dbToY(0) + 3} textAnchor="end" fontSize={9} fill="currentColor" opacity={0.6} fontWeight="bold">0</text>
    </svg>
  );
}
