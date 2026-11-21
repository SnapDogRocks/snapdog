"use client";

import { useEffect, useRef, useCallback } from "react";
import { HugeiconsIcon } from "@hugeicons/react";
import {
  VolumeHighIcon,
  VolumeLowIcon,
  VolumeMute02Icon,
} from "@hugeicons/core-free-icons";
import { useTranslations } from "next-intl";
import { Slider } from "@/components/ui/slider";
import { Button } from "@/components/ui/button";
import { useOptimisticValue } from "@/hooks/useOptimisticValue";

const VOLUME_DEBOUNCE_MS = 50;
const VOLUME_HIGH_THRESHOLD = 50;
const ABSOLUTE_MAX_VOLUME = 100;

function clampVolume(value: number, max: number) {
  return Math.max(0, Math.min(max, value));
}

interface VolumeSliderProps {
  volume: number;
  muted: boolean;
  onVolumeChange: (volume: number) => void;
  onMuteToggle: () => void;
  onUnmute: () => void;
  /** Maximum volume limit (0–100). Shows a red marker and caps the slider. */
  max?: number;
  /** Compact mode for client chips (smaller controls, no value display) */
  compact?: boolean;
  /** Accessible name for this specific zone or speaker volume control. */
  label?: string;
}

export function VolumeSlider({
  volume,
  muted,
  onVolumeChange,
  onMuteToggle,
  onUnmute,
  max = 100,
  compact = false,
  label,
}: VolumeSliderProps) {
  const t = useTranslations("volume");
  const { value: localVolume, setOptimistic, commit, cancel } = useOptimisticValue(volume);
  const timerRef = useRef<ReturnType<typeof setTimeout>>(undefined);
  const pendingVolumeRef = useRef<number | null>(null);
  const lastSentVolumeRef = useRef<number | null>(null);
  const mountedRef = useRef(false);
  const pointerActiveRef = useRef(false);
  // Radix commits only when the value at release differs from the one at pointer-down. A drag
  // that ends where it started (or at the max_volume cap) is committed here instead, so the
  // slider never stays in the dragging state and keeps following the server.
  const committedSinceDownRef = useRef(false);
  const lastValueRef = useRef<number | null>(null);
  const unmuteRequestedRef = useRef(false);
  const effectiveMax = clampVolume(max, ABSOLUTE_MAX_VOLUME);
  const displayVolume = clampVolume(localVolume, effectiveMax);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      clearTimeout(timerRef.current);
      pendingVolumeRef.current = null;
    };
  }, []);

  useEffect(() => {
    if (!muted) unmuteRequestedRef.current = false;
  }, [muted]);

  const volumeIcon = muted
    ? VolumeMute02Icon
    : displayVolume > VOLUME_HIGH_THRESHOLD
      ? VolumeHighIcon
      : VolumeLowIcon;

  const handleChange = useCallback(
    (value: number[]) => {
      if (value[0] === undefined) return;
      const v = clampVolume(value[0], effectiveMax);
      if (pointerActiveRef.current) {
        lastValueRef.current = v;
        setOptimistic(v);
      }
      if (muted && !unmuteRequestedRef.current) {
        unmuteRequestedRef.current = true;
        onUnmute();
      }
      clearTimeout(timerRef.current);
      if (!pointerActiveRef.current) {
        pendingVolumeRef.current = null;
        return;
      }
      pendingVolumeRef.current = v;
      timerRef.current = setTimeout(() => {
        const pendingVolume = pendingVolumeRef.current;
        pendingVolumeRef.current = null;
        if (!mountedRef.current || pendingVolume === null) return;
        lastSentVolumeRef.current = pendingVolume;
        onVolumeChange(pendingVolume);
      }, VOLUME_DEBOUNCE_MS);
    },
    [effectiveMax, muted, onVolumeChange, onUnmute, setOptimistic],
  );

  const handleCommit = useCallback(
    (value: number[]) => {
      if (value[0] === undefined) return;
      const v = clampVolume(value[0], effectiveMax);
      if (muted && !unmuteRequestedRef.current) {
        unmuteRequestedRef.current = true;
        onUnmute();
      }
      committedSinceDownRef.current = true;
      clearTimeout(timerRef.current);
      pendingVolumeRef.current = null;
      commit(v);
      if (lastSentVolumeRef.current !== v) onVolumeChange(v);
      lastSentVolumeRef.current = null;
    },
    [commit, effectiveMax, muted, onUnmute, onVolumeChange],
  );

  const handlePointerCancel = useCallback(() => {
    pointerActiveRef.current = false;
    clearTimeout(timerRef.current);
    pendingVolumeRef.current = null;
    const restoreServerVolume = lastSentVolumeRef.current !== null;
    lastSentVolumeRef.current = null;
    cancel();
    if (restoreServerVolume) onVolumeChange(clampVolume(volume, effectiveMax));
  }, [cancel, effectiveMax, onVolumeChange, volume]);

  const handlePointerDown = useCallback(() => {
    pointerActiveRef.current = true;
    unmuteRequestedRef.current = false;
    committedSinceDownRef.current = false;
    lastValueRef.current = null;
  }, []);

  const handleCommitRef = useRef(handleCommit);
  useEffect(() => { handleCommitRef.current = handleCommit; }, [handleCommit]);

  const handlePointerUp = useCallback(() => {
    pointerActiveRef.current = false;
    // Runs after the Radix pointer-up handler, which commits a changed value synchronously.
    setTimeout(() => {
      const last = lastValueRef.current;
      lastValueRef.current = null;
      if (!mountedRef.current || committedSinceDownRef.current || last === null) return;
      handleCommitRef.current([last]);
    }, 0);
  }, []);

  const iconSize = compact ? 14 : 18;
  const btnSize = compact ? "size-6" : "size-8";

  return (
    <div className={`flex items-center gap-${compact ? "1.5" : "3"} w-full`}>
      <Button
        variant="ghost"
        size="icon"
        onClick={onMuteToggle}
        onDragStart={(e) => { e.preventDefault(); }}
        className={`${btnSize} shrink-0 rounded-full`}
        aria-label={muted ? t("unmute") : t("mute")}
      >
        <HugeiconsIcon icon={volumeIcon} size={iconSize} />
      </Button>
      <div className="relative flex-1 min-w-0">
        <Slider
          value={[muted ? 0 : displayVolume]}
          max={ABSOLUTE_MAX_VOLUME}
          step={1}
          onValueChange={handleChange}
          onValueCommit={handleCommit}
          onPointerDown={handlePointerDown}
          onPointerUp={handlePointerUp}
          onPointerCancel={handlePointerCancel}
          onDragStart={(e: React.DragEvent) => { e.preventDefault(); }}
          className="flex-1 min-w-0"
          aria-label={label ?? t("label")}
          getAriaValueText={(value) => `${Math.round(value)}%`}
        />
        {effectiveMax < ABSOLUTE_MAX_VOLUME && (
          <>
            <div
              className="absolute top-0 h-full w-0.5 bg-red-500/70 rounded-full pointer-events-none"
              style={{ left: `${effectiveMax}%` }}
              role="presentation"
              aria-hidden="true"
            />
            <span className="sr-only">{t("maxVolume", { max: effectiveMax })}</span>
          </>
        )}
      </div>
      <span className={`text-muted-foreground tabular-nums text-right ${compact ? "text-[10px] w-5" : "text-xs w-7"}`}>
        {displayVolume}
      </span>
    </div>
  );
}
