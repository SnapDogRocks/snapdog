import { HugeiconsIcon } from "@hugeicons/react";
import {
  PlayIcon,
  PauseIcon,
  NextIcon,
  PreviousIcon,
} from "@hugeicons/core-free-icons";
import { motion, useReducedMotion } from "framer-motion";
import { useTranslations } from "next-intl";
import { useRef, useCallback } from "react";
import type { ZoneState } from "@/stores/useAppStore";
import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";
import { logApiError } from "@/lib/log-api-error";

/** Duration in ms to trigger stop via long-press. */
const LONG_PRESS_MS = 600;

interface TransportControlsProps {
  zone: ZoneState;
}

export function TransportControls({ zone }: TransportControlsProps) {
  const t = useTranslations("transport");
  const { index, playback, source } = zone;
  const reduced = useReducedMotion();
  const tap = reduced ? {} : { scale: 0.9 };
  const isPlaying = playback === "playing";
  const isIdle = source === "idle";
  const isAirPlay = source === "airplay";
  const isUrl = source === "url";
  const hasNavigation = source === "radio" || source === "subsonic_playlist" || isAirPlay;

  const longPressTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const didLongPress = useRef(false);

  const onPointerDown = useCallback(() => {
    didLongPress.current = false;
    longPressTimer.current = setTimeout(() => {
      didLongPress.current = true;
      api.zones.stop(index).catch(logApiError);
    }, LONG_PRESS_MS);
  }, [index]);

  const clearTimer = useCallback(() => {
    if (longPressTimer.current) {
      clearTimeout(longPressTimer.current);
      longPressTimer.current = null;
    }
  }, []);

  const onClickPlayPause = useCallback(() => {
    if (!didLongPress.current) {
      api.zones[isPlaying ? "pause" : "play"](index).catch(logApiError);
    }
    didLongPress.current = false;
  }, [index, isPlaying]);

  const cmd = (action: string) => {
    switch (action) {
      case "next": api.zones.next(index).catch(logApiError); break;
      case "previous": api.zones.previous(index).catch(logApiError); break;
    }
  };

  return (
    <div className="flex items-center justify-center gap-2">
      <motion.div whileTap={tap}>
        <Button variant="ghost" size="icon" disabled={isIdle || isAirPlay || isUrl || !hasNavigation} onClick={() => cmd("previous")} className="size-10 rounded-full" aria-label={t("previous")}>
          <HugeiconsIcon icon={PreviousIcon} size={20} />
        </Button>
      </motion.div>
      <motion.div whileTap={tap}>
        <Button
          variant="default"
          size="icon"
          disabled={isIdle}
          onPointerDown={onPointerDown}
          onPointerUp={clearTimer}
          onPointerLeave={clearTimer}
          onContextMenu={(e) => e.preventDefault()}
          onClick={onClickPlayPause}
          className="size-12 rounded-full"
          aria-label={isPlaying ? t("pause") : t("play")}
        >
          <HugeiconsIcon icon={isPlaying ? PauseIcon : PlayIcon} size={24} />
        </Button>
      </motion.div>
      <motion.div whileTap={tap}>
        <Button variant="ghost" size="icon" disabled={isIdle || isAirPlay || isUrl || !hasNavigation} onClick={() => cmd("next")} className="size-10 rounded-full" aria-label={t("next")}>
          <HugeiconsIcon icon={NextIcon} size={20} />
        </Button>
      </motion.div>
    </div>
  );
}
