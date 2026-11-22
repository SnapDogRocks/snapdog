import type { EqConfig } from "./types";

/** Which EQ an event belongs to: a zone EQ, a client EQ or a client's speaker correction. */
export type EqTarget = `zone:${number}` | `client:${number}` | `speaker:${number}`;

type Listener = (config: EqConfig) => void;
const listeners = new Map<EqTarget, Set<Listener>>();

/** Forward an EQ change reported by the server to open editors of that target. */
export function publishEqChange(target: EqTarget, config: EqConfig) {
  listeners.get(target)?.forEach((listener) => { listener(config); });
}

/** Listen for server-side changes of one EQ. Returns the unsubscribe function. */
export function subscribeEqChanges(target: EqTarget, listener: Listener): () => void {
  const set = listeners.get(target) ?? new Set<Listener>();
  set.add(listener);
  listeners.set(target, set);
  return () => {
    set.delete(listener);
    if (set.size === 0) listeners.delete(target);
  };
}
