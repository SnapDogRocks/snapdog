"use client";

import { useEffect } from "react";
import { useAppStore } from "@/stores/useAppStore";
import { api } from "@/lib/api";
import { logApiError } from "@/lib/log-api-error";

/** Volume step per arrow key press. */
const VOLUME_STEP = 5;

const INTERACTIVE_TARGETS = [
  "input",
  "textarea",
  "select",
  "button",
  "a[href]",
  "summary",
  "[role='button']",
  "[role='dialog']",
  "[role='alertdialog']",
  "[role='slider']",
  "[role='spinbutton']",
  "[role='textbox']",
  "[role='combobox']",
  "[role='listbox']",
  "[role='option']",
  "[role='menu']",
  "[role='menuitem']",
  "[role='tab']",
  "[contenteditable]:not([contenteditable='false'])",
  "[tabindex]:not([tabindex='-1'])",
].join(",");

function isInteractiveTarget(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  if (target instanceof HTMLElement && target.isContentEditable) return true;
  return target.closest(INTERACTIVE_TARGETS) !== null;
}

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
      if (e.defaultPrevented || e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
      if (isInteractiveTarget(e.target)) return;

      const zone = zones.get(selectedZone);
      if (!zone) return;

      switch (e.key) {
        case " ":
          if (zone.source === "idle") return;
          e.preventDefault();
          api.zones[zone.playback === "playing" ? "pause" : "play"](zone.index).catch(logApiError);
          break;
        case "ArrowLeft":
          if (!zone.track?.can_prev) return;
          e.preventDefault();
          api.zones.previous(zone.index).catch(logApiError);
          break;
        case "ArrowRight":
          if (!zone.track?.can_next) return;
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
