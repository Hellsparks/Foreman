import { useCallback, useEffect, useRef, useState } from "react";
import type { ServerEvent } from "../api/types/ServerEvent";

export type ConnectionState = "connecting" | "connected" | "disconnected";

export interface EventStreamState {
  connectionState: ConnectionState;
  lastHeartbeat: Date | null;
}

const MIN_RECONNECT_MS = 1_000;
const MAX_RECONNECT_MS = 30_000;

function wsUrl(): string {
  const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
  return `${proto}//${window.location.host}/ws/events`;
}

/**
 * Connects to the server's `/ws/events` WebSocket and returns connection state
 * and the timestamp of the last heartbeat received.
 *
 * Reconnects automatically with exponential backoff (1 s → 30 s).
 */
export function useEventStream(): EventStreamState {
  const [state, setState] = useState<EventStreamState>({
    connectionState: "connecting",
    lastHeartbeat: null,
  });

  const wsRef = useRef<WebSocket | null>(null);
  const reconnectDelayRef = useRef(MIN_RECONNECT_MS);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Track whether the component is still mounted so we don't setState after
  // unmount during a reconnect timeout.
  const mountedRef = useRef(true);

  const connect = useCallback(() => {
    if (!mountedRef.current) return;

    const ws = new WebSocket(wsUrl());
    wsRef.current = ws;
    setState((s) => ({ ...s, connectionState: "connecting" }));

    ws.onopen = () => {
      if (!mountedRef.current) return;
      reconnectDelayRef.current = MIN_RECONNECT_MS; // reset backoff on success
      setState((s) => ({ ...s, connectionState: "connected" }));
    };

    ws.onmessage = (ev: MessageEvent<string>) => {
      if (!mountedRef.current) return;
      try {
        const event = JSON.parse(ev.data) as ServerEvent;
        if (event.type === "heartbeat") {
          setState((s) => ({
            ...s,
            lastHeartbeat: new Date(event.timestamp),
          }));
        }
      } catch {
        // Ignore malformed messages.
      }
    };

    ws.onclose = () => {
      if (!mountedRef.current) return;
      setState((s) => ({ ...s, connectionState: "disconnected" }));
      // Exponential back-off.
      const delay = reconnectDelayRef.current;
      reconnectDelayRef.current = Math.min(delay * 2, MAX_RECONNECT_MS);
      timerRef.current = setTimeout(connect, delay);
    };

    ws.onerror = () => {
      // onerror is always followed by onclose; let onclose handle reconnect.
      ws.close();
    };
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    connect();
    return () => {
      mountedRef.current = false;
      if (timerRef.current !== null) clearTimeout(timerRef.current);
      wsRef.current?.close();
    };
  }, [connect]);

  return state;
}
