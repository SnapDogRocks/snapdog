"use client";
import { useCallback, useSyncExternalStore } from "react";

/** Only the active responsive composition mounts its interactive children. */
export function useMediaQuery(query: string) {
  const subscribe = useCallback((notify: () => void) => {
    const media = window.matchMedia(query);
    media.addEventListener("change", notify);
    return () => { media.removeEventListener("change", notify); };
  }, [query]);
  const snapshot = useCallback(() => window.matchMedia(query).matches, [query]);
  return useSyncExternalStore(subscribe, snapshot, () => false);
}
