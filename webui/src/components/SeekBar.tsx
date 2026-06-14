"use client";

import { useEffect, useRef, useCallback, useReducer } from "react";
import { useTranslations } from "next-intl";
import { Slider } from "@/components/ui/slider";
import { api } from "@/lib/api";
import { logApiError } from "@/lib/log-api-error";
import { formatTime } from "@/lib/format-time";
import type { ZoneState } from "@/stores/useAppStore";

const INTERPOLATION_INTERVAL_MS = 250;
const SEEK_STEP_MS = 1000;
/** Safety timeout: if server doesn't confirm seek within this time, resume sync */
const SEEK_TIMEOUT_MS = 5000;
/** Server position must be within this range of target to count as confirmed.
 *  Accounts for decoder granularity (AAC frames ~23ms, FLAC ~20ms) and network jitter. */
const SEEK_CONFIRM_TOLERANCE_MS = 2000;

// ── State machine ─────────────────────────────────────────────

type SeekState =
  | { type: "synced"; position: number }
  | { type: "dragging"; position: number }
  | { type: "seeking"; target: number; sentAt: number };

type SeekAction =
  | { type: "server_update"; position: number }
  | { type: "drag"; position: number }
  | { type: "commit"; target: number }
  | { type: "tick"; delta: number; duration: number }
  | { type: "track_change" }
  | { type: "timeout" };

function seekReducer(state: SeekState, action: SeekAction): SeekState {
  switch (action.type) {
    case "server_update":
      switch (state.type) {
        case "synced":
          return { type: "synced", position: action.position };
        case "dragging":
          return state; // ignore server while dragging
        case "seeking":
          // Server confirmed: position is close to the seek target
          if (Math.abs(action.position - state.target) <= SEEK_CONFIRM_TOLERANCE_MS) {
            return { type: "synced", position: action.position };
          }
          return state; // still waiting
      }
      break; // unreachable but satisfies TS
    case "drag":
      return { type: "dragging", position: action.position };
    case "commit":
      return { type: "seeking", target: action.target, sentAt: Date.now() };
    case "tick":
      if (state.type === "synced") {
        const next = action.duration > 0
          ? Math.min(state.position + action.delta, action.duration)
          : state.position + action.delta;
        return { type: "synced", position: next };
      }
      return state;
    case "track_change":
      return { type: "synced", position: 0 };
    case "timeout":
      if (state.type === "seeking") {
        return { type: "synced", position: state.target };
      }
      return state;
  }
  return state;
}

// ── Component ─────────────────────────────────────────────────

export function SeekBar({ zone }: { zone: ZoneState }) {
  const t = useTranslations("seek");
  const track = zone.track;
  const duration = track?.duration_ms ?? 0;
  const serverPosition = track?.position_ms ?? 0;
  const isPlaying = zone.playback === "playing";
  const isIdle = zone.source === "idle" || !track;
  const canSeek = track?.seekable ?? false;
  const bufferedMs = zone.buffered_ms ?? null;

  const [state, dispatch] = useReducer(seekReducer, { type: "synced", position: serverPosition });

  const trackKey = `${track?.title}-${track?.artist}`;
  const lastTrackRef = useRef(trackKey);

  // Track change detection
  useEffect(() => {
    if (trackKey !== lastTrackRef.current) {
      lastTrackRef.current = trackKey;
      dispatch({ type: "track_change" });
    }
  }, [trackKey]);

  // Server position sync
  useEffect(() => {
    dispatch({ type: "server_update", position: serverPosition });
  }, [serverPosition]);

  // Client-side interpolation
  useEffect(() => {
    if (!isPlaying || state.type !== "synced" || isIdle) return;
    const interval = setInterval(() => {
      dispatch({ type: "tick", delta: INTERPOLATION_INTERVAL_MS, duration });
    }, INTERPOLATION_INTERVAL_MS);
    return () => clearInterval(interval);
  }, [isPlaying, state.type, isIdle, duration]);

  // Seek timeout safety net
  useEffect(() => {
    if (state.type !== "seeking") return;
    const remaining = SEEK_TIMEOUT_MS - (Date.now() - state.sentAt);
    if (remaining <= 0) {
      dispatch({ type: "timeout" });
      return;
    }
    const timer = setTimeout(() => dispatch({ type: "timeout" }), remaining);
    return () => clearTimeout(timer);
  }, [state]);

  const handleSeek = useCallback(
    (value: number[]) => {
      if (!canSeek) return;
      dispatch({ type: "drag", position: value[0] });
    },
    [canSeek],
  );

  const handleSeekCommit = useCallback(
    (value: number[]) => {
      if (!canSeek) return;
      dispatch({ type: "commit", target: value[0] });
      api.zones.seekPosition(zone.index, value[0]).catch(logApiError);
    },
    [zone.index, canSeek],
  );

  const displayPosition = state.type === "seeking" ? state.target : state.position;
  const isEndless = duration === 0 && !isIdle && isPlaying;

  if (isIdle) return (
    <div className="w-full space-y-1">
      <Slider value={[0]} max={1} step={1} disabled className="w-full" aria-label={t("label")} />
      <div className="flex justify-between text-[10px] text-muted-foreground tabular-nums">
        <span>--:--</span>
        <span>--:--</span>
      </div>
    </div>
  );

  return (
    <div className="w-full space-y-1">
      <div className="relative">
        {bufferedMs != null && duration > 0 && bufferedMs < duration && (
          <div
            className="absolute top-1/2 -translate-y-1/2 left-0 h-3 rounded-4xl bg-primary/20 pointer-events-none"
            style={{ width: `${Math.min((bufferedMs / duration) * 100, 100)}%` }}
          />
        )}
        <Slider
          value={isEndless ? [0] : [displayPosition]}
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
        <span>{formatTime(displayPosition)}</span>
        <span>{duration > 0 ? formatTime(duration) : "∞"}</span>
      </div>
    </div>
  );
}
