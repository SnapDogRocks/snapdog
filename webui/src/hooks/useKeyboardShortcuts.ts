"use client";

import { useEffect } from "react";
import { useAppStore } from "@/stores/useAppStore";
import { api } from "@/lib/api";
import { logApiError } from "@/lib/log-api-error";

/** Volume step per arrow key press. */
const VOLUME_STEP = 5;

/**
 * Global keyboard shortcuts for the active zone.
 * Space = Play/Pause, ←/→ = Prev/Next, ↑/↓ = Volume.
 * Only active when no input/textarea/select is focused.
 */
export function useKeyboardShortcuts() {
  const zones = useAppStore((s) => s.zones);
  const selectedZone = useAppStore((s) => s.selectedZone);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      // Don't intercept when typing in inputs
      const tag = (e.target as HTMLElement | null)?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;

      const zone = zones.get(selectedZone);
      if (!zone) return;

      switch (e.key) {
        case " ":
          e.preventDefault();
          api.zones[zone.playback === "playing" ? "pause" : "play"](zone.index).catch(logApiError);
          break;
        case "ArrowLeft":
          e.preventDefault();
          api.zones.previous(zone.index).catch(logApiError);
          break;
        case "ArrowRight":
          e.preventDefault();
          api.zones.next(zone.index).catch(logApiError);
          break;
        case "ArrowUp":
          e.preventDefault();
          api.zones.setVolume(zone.index, Math.min(100, zone.volume + VOLUME_STEP)).catch(logApiError);
          break;
        case "ArrowDown":
          e.preventDefault();
          api.zones.setVolume(zone.index, Math.max(0, zone.volume - VOLUME_STEP)).catch(logApiError);
          break;
        case "m":
          api.zones.toggleMute(zone.index).catch(logApiError);
          break;
      }
    };

    window.addEventListener("keydown", handler);
    return () => { window.removeEventListener("keydown", handler); };
  }, [zones, selectedZone]);
}
