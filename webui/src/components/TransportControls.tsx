import { HugeiconsIcon } from "@hugeicons/react";
import {
  PlayIcon,
  PauseIcon,
  NextIcon,
  PreviousIcon,
  StopIcon,
} from "@hugeicons/core-free-icons";
import { motion, useReducedMotion } from "framer-motion";
import { useTranslations } from "next-intl";
import { useRef, useCallback, useEffect, useState } from "react";
import type { ZoneState } from "@/stores/useAppStore";
import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";
import { logApiError } from "@/lib/log-api-error";

/** Duration in ms to trigger stop via long-press. */
const LONG_PRESS_MS = 600;
/** Duration in ms before showing the long-press hint. */
const LONG_PRESS_HINT_MS = 400;

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
  const canNext = zone.track?.can_next ?? false;
  const canPrev = zone.track?.can_prev ?? false;

  const longPressTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const hintTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const didLongPress = useRef(false);
  const [pressing, setPressing] = useState(false);
  const [pendingActions, setPendingActions] = useState<Set<string>>(() => new Set());
  const pendingActionsRef = useRef(pendingActions);
  const mountedRef = useRef(false);

  const runAction = useCallback((actionName: string, action: () => Promise<unknown>) => {
    if (pendingActionsRef.current.has(actionName)) return;
    const pending = new Set(pendingActionsRef.current);
    pending.add(actionName);
    pendingActionsRef.current = pending;
    setPendingActions(pending);
    void action().catch(logApiError).finally(() => {
      const nextPending = new Set(pendingActionsRef.current);
      nextPending.delete(actionName);
      pendingActionsRef.current = nextPending;
      if (mountedRef.current) setPendingActions(nextPending);
    });
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      if (longPressTimer.current) clearTimeout(longPressTimer.current);
      if (hintTimer.current) clearTimeout(hintTimer.current);
      longPressTimer.current = null;
      hintTimer.current = null;
    };
  }, []);

  const onPointerDown = useCallback(() => {
    didLongPress.current = false;
    hintTimer.current = setTimeout(() => { setPressing(true); }, LONG_PRESS_HINT_MS);
    longPressTimer.current = setTimeout(() => {
      didLongPress.current = true;
      setPressing(false);
      runAction("stop", () => api.zones.stop(index));
    }, LONG_PRESS_MS);
  }, [index, runAction]);

  const clearTimer = useCallback(() => {
    if (longPressTimer.current) {
      clearTimeout(longPressTimer.current);
      longPressTimer.current = null;
    }
    if (hintTimer.current) {
      clearTimeout(hintTimer.current);
      hintTimer.current = null;
    }
    setPressing(false);
  }, []);

  const onPointerCancel = useCallback(() => {
    clearTimer();
    didLongPress.current = false;
  }, [clearTimer]);

  const onClickPlayPause = useCallback(() => {
    if (!didLongPress.current) {
      runAction("play-pause", () => api.zones[isPlaying ? "pause" : "play"](index));
    }
    didLongPress.current = false;
  }, [index, isPlaying, runAction]);

  const cmd = (action: "next" | "previous") => {
    runAction(action, () => api.zones[action](index));
  };

  return (
    <div className="flex items-center justify-center gap-2">
      <motion.div whileTap={tap}>
        <Button variant="ghost" size="icon" disabled={isIdle || !canPrev || pendingActions.has("previous")} onClick={() => { cmd("previous"); }} className="size-10 rounded-full" aria-label={t("previous")}>
          <HugeiconsIcon icon={PreviousIcon} size={20} />
        </Button>
      </motion.div>
      <motion.div whileTap={tap}>
        <Button
          variant="default"
          size="icon"
          disabled={isIdle || pendingActions.has("play-pause")}
          onPointerDown={onPointerDown}
          onPointerUp={clearTimer}
          onPointerLeave={clearTimer}
          onPointerCancel={onPointerCancel}
          onContextMenu={(e) => { e.preventDefault(); }}
          onClick={onClickPlayPause}
          className={`size-12 rounded-full transition-transform ${pressing ? "scale-90" : ""} ${isPlaying ? "shadow-[0_0_16px_rgba(225,136,46,0.4)]" : ""}`}
          aria-label={isPlaying ? t("pause") : t("play")}
        >
          <HugeiconsIcon icon={isPlaying ? PauseIcon : PlayIcon} size={24} />
        </Button>
      </motion.div>
      <motion.div whileTap={tap}>
        <Button variant="ghost" size="icon" disabled={isIdle || !canNext || pendingActions.has("next")} onClick={() => { cmd("next"); }} className="size-10 rounded-full" aria-label={t("next")}>
          <HugeiconsIcon icon={NextIcon} size={20} />
        </Button>
      </motion.div>
      <motion.div whileTap={tap}>
        <Button
          variant="ghost"
          size="icon"
          disabled={isIdle || pendingActions.has("stop")}
          onClick={() => { runAction("stop", () => api.zones.stop(index)); }}
          className="size-10 rounded-full"
          aria-label={t("stop")}
          title={t("stop")}
        >
          <HugeiconsIcon icon={StopIcon} size={18} />
        </Button>
      </motion.div>
    </div>
  );
}
