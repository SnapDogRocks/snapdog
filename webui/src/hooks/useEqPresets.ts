import { useEffect, useState } from "react";
import { api } from "@/lib/api";
import { logApiError } from "@/lib/log-api-error";

let cached: Promise<string[]> | null = null;

/** Forget the loaded list (tests). */
export function resetEqPresetsCache(): void {
  cached = null;
}

/**
 * The preset names the server accepts, in its order, from `GET /eq/presets`. Loaded once
 * per page; empty until loaded and after a failed load (which shows a toast).
 */
export function useEqPresets(): string[] {
  const [names, setNames] = useState<string[]>([]);
  useEffect(() => {
    let alive = true;
    cached ??= api.eq.presets().then((list) => list.map((p) => p.name));
    cached.then((list) => { if (alive) setNames(list); }).catch((e: unknown) => {
      cached = null;
      logApiError(e);
    });
    return () => { alive = false; };
  }, []);
  return names;
}
