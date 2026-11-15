"use client";

import { useEffect, useRef, useState } from "react";
import type { WsNotification } from "@/lib/types";
import { getApiKey } from "@/lib/auth";

const BACKOFF_STEPS = [1_000, 2_500, 5_000, 10_000, 15_000];

export function useWebSocket(
  onNotification: (n: WsNotification) => void,
  onReconnect?: () => void,
  enabled = true,
) {
  const [isConnected, setIsConnected] = useState(false);
  const [serverGoingAway, setServerGoingAway] = useState(false);
  const [retryIn, setRetryIn] = useState(0);
  const wsRef = useRef<WebSocket | null>(null);
  const onNotifRef = useRef(onNotification);
  const onReconnectRef = useRef(onReconnect);

  useEffect(() => {
    onNotifRef.current = onNotification;
    onReconnectRef.current = onReconnect;
  }, [onNotification, onReconnect]);

  useEffect(() => {
    if (!enabled) return;

    let disposed = false;
    let socket: WebSocket | null = null;
    let attempt = 0;
    let wasConnected = false;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
    let countdownTimer: ReturnType<typeof setTimeout> | null = null;

    const clearTimers = () => {
      if (reconnectTimer !== null) clearTimeout(reconnectTimer);
      if (countdownTimer !== null) clearTimeout(countdownTimer);
      reconnectTimer = null;
      countdownTimer = null;
    };

    const closeSocket = () => {
      const current = socket;
      socket = null;
      if (!current) return;
      current.onopen = null;
      current.onmessage = null;
      current.onclose = null;
      current.onerror = null;
      if (current.readyState < WebSocket.CLOSING) current.close();
      if (wsRef.current === current) wsRef.current = null;
    };

    const scheduleReconnect = () => {
      if (disposed) return;
      const maxBackoff = BACKOFF_STEPS[BACKOFF_STEPS.length - 1] ?? 15_000;
      const delay = BACKOFF_STEPS[Math.min(attempt, BACKOFF_STEPS.length - 1)] ?? maxBackoff;
      attempt++;
      const deadline = Date.now() + delay;

      const updateCountdown = () => {
        if (disposed) return;
        const remaining = Math.max(0, deadline - Date.now());
        setRetryIn(Math.ceil(remaining / 1_000));
        if (remaining > 0) {
          countdownTimer = setTimeout(updateCountdown, Math.min(1_000, remaining));
        } else {
          countdownTimer = null;
        }
      };

      updateCountdown();
      reconnectTimer = setTimeout(() => {
        reconnectTimer = null;
        if (countdownTimer !== null) clearTimeout(countdownTimer);
        countdownTimer = null;
        setRetryIn(0);
        connect();
      }, delay);
    };

    const connect = () => {
      if (disposed) return;
      const proto = location.protocol === "https:" ? "wss:" : "ws:";
      const key = getApiKey();
      const wsUrl = key
        ? `${proto}//${location.host}/ws?token=${encodeURIComponent(key)}`
        : `${proto}//${location.host}/ws`;

      let ws: WebSocket;
      try {
        ws = new WebSocket(wsUrl);
      } catch {
        socket = null;
        scheduleReconnect();
        return;
      }

      socket = ws;
      wsRef.current = ws;

      ws.onopen = () => {
        if (disposed || socket !== ws) return;
        clearTimers();
        const isReconnect = wasConnected;
        wasConnected = true;
        setIsConnected(true);
        setServerGoingAway(false);
        setRetryIn(0);
        attempt = 0;
        if (isReconnect) onReconnectRef.current?.();
      };

      ws.onmessage = (event) => {
        if (disposed || socket !== ws) return;
        try {
          // The server sends text frames; MessageEvent types `data` as `any`.
          const data = JSON.parse(event.data as string) as WsNotification;
          onNotifRef.current(data);
        } catch {
          /* Ignore malformed messages. */
        }
      };

      ws.onclose = (event) => {
        if (disposed || socket !== ws) return;
        socket = null;
        if (wsRef.current === ws) wsRef.current = null;
        setIsConnected(false);
        setServerGoingAway(event.code === 1001);
        scheduleReconnect();
      };

      ws.onerror = () => {
        if (!disposed && socket === ws) ws.close();
      };
    };

    const onAuthChanged = () => {
      if (disposed) return;
      clearTimers();
      closeSocket();
      attempt = 0;
      wasConnected = false;
      setIsConnected(false);
      setRetryIn(0);
      connect();
    };

    window.addEventListener("snapdog:auth-changed", onAuthChanged);
    connect();

    return () => {
      disposed = true;
      window.removeEventListener("snapdog:auth-changed", onAuthChanged);
      clearTimers();
      closeSocket();
      setIsConnected(false);
      setServerGoingAway(false);
      setRetryIn(0);
    };
  }, [enabled]);

  return {
    isConnected: enabled && isConnected,
    serverGoingAway: enabled && serverGoingAway,
    retryIn: enabled ? retryIn : 0,
  };
}
