"use client";

import { useState, useRef, useEffect, useLayoutEffect, useCallback } from "react";

/**
 * Optimistic UI pattern for server-controlled values.
 *
 * Returns a local value that:
 * - Follows the server prop when idle
 * - Holds the user's committed value until the server confirms (within tolerance)
 * - Falls back to server value after a timeout (prevents permanent desync)
 *
 * @param serverValue - The authoritative value from the server
 * @param tolerance - How close the server value must be to count as "confirmed" (default: 1)
 * @param timeoutMs - Max time to hold the optimistic value before accepting server (default: 2000)
 */
export function useOptimisticValue(
  serverValue: number,
  { tolerance = 1, timeoutMs = 2000 } = {},
) {
  const [localValue, setLocalValue] = useState(serverValue);
  const [pending, setPending] = useState(false);
  const committedRef = useRef<number | null>(null);
  const serverValueRef = useRef(serverValue);
  const draggingRef = useRef(false);
  const timeoutRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  useLayoutEffect(() => {
    serverValueRef.current = serverValue;
  }, [serverValue]);

  // Sync from server when not waiting for confirmation
  useEffect(() => {
    if (draggingRef.current) return;
    if (committedRef.current === null) {
      setLocalValue(serverValue);
    } else if (Math.abs(serverValue - committedRef.current) <= tolerance) {
      // Server confirmed
      committedRef.current = null;
      setPending(false);
      setLocalValue(serverValue);
    }
  }, [serverValue, tolerance]);

  // Cleanup timeout on unmount
  useEffect(() => () => { clearTimeout(timeoutRef.current); }, []);

  /** Call during drag/interaction — updates local immediately, no server call. */
  const setOptimistic = useCallback((value: number) => {
    clearTimeout(timeoutRef.current);
    committedRef.current = null;
    setPending(false);
    draggingRef.current = true;
    setLocalValue(value);
  }, []);

  /** Call if an interaction is cancelled before commit. */
  const cancel = useCallback(() => {
    clearTimeout(timeoutRef.current);
    committedRef.current = null;
    draggingRef.current = false;
    setPending(false);
    setLocalValue(serverValueRef.current);
  }, []);

  /** Call on commit (release) — holds value until server confirms. */
  const commit = useCallback(
    (value: number) => {
      draggingRef.current = false;
      setLocalValue(value);
      committedRef.current = value;
      setPending(true);
      // Safety fallback: release after timeout
      clearTimeout(timeoutRef.current);
      timeoutRef.current = setTimeout(() => {
        committedRef.current = null;
        setPending(false);
        setLocalValue(serverValueRef.current);
      }, timeoutMs);
    },
    [timeoutMs],
  );

  return { value: localValue, setOptimistic, commit, cancel, pending };
}
