"use client";

import { useState, useEffect, useRef, useCallback } from "react";
import { useTranslations } from "next-intl";
import { Slider } from "@/components/ui/slider";
import { api } from "@/lib/api";
import { logApiError } from "@/lib/log-api-error";
import { formatTime } from "@/lib/format-time";
import type { ZoneState } from "@/stores/useAppStore";

const INTERPOLATION_INTERVAL_MS = 250;
const SEEK_STEP_MS = 1000;

export function SeekBar({ zone }: { zone: ZoneState }) {
  const t = useTranslations("seek");
  const track = zone.track;
  const duration = track?.duration_ms ?? 0;
  const serverPosition = track?.position_ms ?? 0;
  const isPlaying = zone.playback === "playing";
  const isIdle = zone.source === "idle" || !track;
  const canSeek = track?.seekable ?? false;
  const bufferedMs = zone.buffered_ms ?? null;

  const [localPosition, setLocalPosition] = useState(serverPosition);
  const [dragging, setDragging] = useState(false);
  const lastServerRef = useRef(serverPosition);

  const trackKey = `${track?.title}-${track?.artist}`;
  const lastTrackRef = useRef(trackKey);

  // Sync from server when position changes externally, or reset on track/playback change
  useEffect(() => {
    if (trackKey !== lastTrackRef.current) {
      setLocalPosition(0);
      lastTrackRef.current = trackKey;
      lastServerRef.current = 0;
    } else if (!dragging) {
      setLocalPosition(serverPosition);
      lastServerRef.current = serverPosition;
    }
  }, [serverPosition, dragging, trackKey, isPlaying]);

  const isEndless = duration === 0 && !isIdle && isPlaying;

  // Client-side interpolation: tick forward every 250ms while playing
  useEffect(() => {
    if (!isPlaying || dragging || isIdle) return;
    const interval = setInterval(() => {
      setLocalPosition((prev) => duration > 0 ? Math.min(prev + INTERPOLATION_INTERVAL_MS, duration) : prev + INTERPOLATION_INTERVAL_MS);
    }, INTERPOLATION_INTERVAL_MS);
    return () => clearInterval(interval);
  }, [isPlaying, dragging, isIdle, duration]);

  const handleSeek = useCallback(
    (value: number[]) => {
      if (!canSeek) return;
      setLocalPosition(value[0]);
      setDragging(true);
    },
    [canSeek],
  );

  const handleSeekCommit = useCallback(
    (value: number[]) => {
      if (!canSeek) return;
      setDragging(false);
      lastServerRef.current = value[0];
      api.zones.seekPosition(zone.index, value[0]).catch(logApiError);
    },
    [zone.index, canSeek],
  );

  if (isIdle) return (
    <div className="w-full sm:max-w-xs space-y-1">
      <Slider value={[0]} max={1} step={1} disabled className="w-full" aria-label={t("label")} />
      <div className="flex justify-between text-[10px] text-muted-foreground tabular-nums">
        <span>--:--</span>
        <span>--:--</span>
      </div>
    </div>
  );

  return (
    <div className="w-full sm:max-w-xs space-y-1">
      <div className="relative">
        {/* Buffer progress bar — behind the slider */}
        {bufferedMs != null && duration > 0 && bufferedMs < duration && (
          <div
            className="absolute top-1/2 -translate-y-1/2 left-0 h-3 rounded-4xl bg-primary/20 pointer-events-none"
            style={{ width: `${Math.min((bufferedMs / duration) * 100, 100)}%` }}
          />
        )}
        <Slider
          value={isEndless ? [0] : [localPosition]}
          max={isEndless ? 1 : (duration || 1)}
          step={SEEK_STEP_MS}
          onValueChange={handleSeek}
          onValueCommit={handleSeekCommit}
          disabled={!canSeek}
          className="w-full relative"
          aria-label={t("label")}
        />
      </div>
      <div className="flex justify-between text-[10px] text-muted-foreground tabular-nums">
        <span>{formatTime(localPosition)}</span>
        <span>{duration > 0 ? formatTime(duration) : "∞"}</span>
      </div>
    </div>
  );
}
