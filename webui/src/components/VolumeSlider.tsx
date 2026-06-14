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
}

export function VolumeSlider({
  volume,
  muted,
  onVolumeChange,
  onMuteToggle,
  onUnmute,
  max = 100,
  compact = false,
}: VolumeSliderProps) {
  const t = useTranslations("volume");
  const { value: localVolume, setOptimistic, commit } = useOptimisticValue(volume);
  const timerRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  // Clean up debounce timer on unmount
  useEffect(() => () => clearTimeout(timerRef.current), []);

  const volumeIcon = muted
    ? VolumeMute02Icon
    : localVolume > VOLUME_HIGH_THRESHOLD
      ? VolumeHighIcon
      : VolumeLowIcon;

  const handleChange = useCallback(
    (value: number[]) => {
      const v = value[0];
      setOptimistic(v);
      if (muted) onUnmute();
      clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => onVolumeChange(v), VOLUME_DEBOUNCE_MS);
    },
    [muted, onVolumeChange, onUnmute, setOptimistic],
  );

  const handleCommit = useCallback(
    (value: number[]) => {
      clearTimeout(timerRef.current);
      commit(value[0]);
      onVolumeChange(value[0]);
    },
    [commit, onVolumeChange],
  );

  const iconSize = compact ? 14 : 18;
  const btnSize = compact ? "size-6" : "size-8";

  return (
    <div
      className={`flex items-center gap-${compact ? "1.5" : "3"} w-full`}
      onWheel={(e) => {
        e.preventDefault();
        const delta = e.deltaY < 0 ? 5 : -5;
        onVolumeChange(Math.max(0, Math.min(100, volume + delta)));
      }}
    >
      <Button
        variant="ghost"
        size="icon"
        onClick={onMuteToggle}
        onDragStart={(e) => e.preventDefault()}
        className={`${btnSize} shrink-0 rounded-full`}
        aria-label={muted ? t("unmute") : t("mute")}
      >
        <HugeiconsIcon icon={volumeIcon} size={iconSize} />
      </Button>
      <div className="relative flex-1 min-w-0">
        <Slider
          value={[muted ? 0 : localVolume]}
          max={max}
          step={1}
          onValueChange={handleChange}
          onValueCommit={handleCommit}
          onDragStart={(e: React.DragEvent) => e.preventDefault()}
          className="flex-1 min-w-0"
          aria-label={t("label")}
        />
        {max < 100 && (
          <>
            <div
              className="absolute top-0 h-full w-0.5 bg-red-500/70 rounded-full pointer-events-none"
              style={{ left: `${max}%` }}
              role="presentation"
              aria-hidden="true"
            />
            <span className="sr-only">{t("maxVolume", { max })}</span>
          </>
        )}
      </div>
      <span className={`text-muted-foreground tabular-nums text-right ${compact ? "text-[10px] w-5" : "text-xs w-7"}`}>
        {localVolume}
      </span>
    </div>
  );
}
