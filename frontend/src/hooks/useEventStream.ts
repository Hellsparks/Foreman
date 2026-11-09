import { useCallback, useEffect, useRef, useState } from "react";
import type { PrinterStatus } from "../api/types/PrinterStatus";
import { onConnectionChange, wsUrl as serverWsUrl } from "../api/conn";

export type ConnectionState = "connecting" | "connected" | "disconnected";

export interface EventStreamState {
  connectionState: ConnectionState;
  lastHeartbeat: Date | null;
}

/** Parsed printer_status event from the WS bus. */
export interface PrinterStatusEvent {
  printer_id: number;
  status: PrinterStatus;
}

/** Parsed printer_connection event from the WS bus. */
export interface PrinterConnectionEvent {
  printer_id: number;
  event: { kind: string; active_path: string | null };
}

export type EventCallback = (event: {
  type: string;
  [key: string]: unknown;
}) => void;

const MIN_RECONNECT_MS = 1_000;
const MAX_RECONNECT_MS = 30_000;

function wsUrl(): string {
  return serverWsUrl("/ws/events");
}

/**
 * Connects to the server's `/ws/events` WebSocket and returns connection state
 * and the timestamp of the last heartbeat received.
 *
 * Optionally accepts a callback that fires for every non-heartbeat event.
 *
 * Reconnects automatically with exponential backoff (1 s → 30 s).
 */
export function useEventStream(onEvent?: EventCallback): EventStreamState {
  const [state, setState] = useState<EventStreamState>({
    connectionState: "connecting",
    lastHeartbeat: null,
  });

  const onEventRef = useRef(onEvent);
  onEventRef.current = onEvent;

  const wsRef = useRef<WebSocket | null>(null);
  const reconnectDelayRef = useRef(MIN_RECONNECT_MS);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const mountedRef = useRef(true);

  const connect = useCallback(() => {
    if (!mountedRef.current) return;

    const ws = new WebSocket(wsUrl());
    wsRef.current = ws;
    setState((s) => ({ ...s, connectionState: "connecting" }));

    ws.onopen = () => {
      if (!mountedRef.current) return;
      reconnectDelayRef.current = MIN_RECONNECT_MS;
      setState((s) => ({ ...s, connectionState: "connected" }));

      // Subscribe to printer events.
      ws.send(
        JSON.stringify({
          topics: ["printer.status", "printer.connection"],
          printer_ids: [],
        })
      );
    };

    ws.onmessage = (ev: MessageEvent<string>) => {
      if (!mountedRef.current) return;
      try {
        const event = JSON.parse(ev.data) as { type: string; [k: string]: unknown };
        if (event.type === "heartbeat") {
          setState((s) => ({
            ...s,
            lastHeartbeat: new Date(event.timestamp as string),
          }));
        } else {
          onEventRef.current?.(event);
        }
      } catch {
        // Ignore malformed messages.
      }
    };

    ws.onclose = () => {
      if (!mountedRef.current) return;
      setState((s) => ({ ...s, connectionState: "disconnected" }));
      const delay = reconnectDelayRef.current;
      reconnectDelayRef.current = Math.min(delay * 2, MAX_RECONNECT_MS);
      timerRef.current = setTimeout(connect, delay);
    };

    ws.onerror = () => {
      ws.close();
    };
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    connect();
    // Failover moved to another address: drop the old socket and reconnect at once.
    const off = onConnectionChange(() => {
      if (timerRef.current !== null) clearTimeout(timerRef.current);
      reconnectDelayRef.current = MIN_RECONNECT_MS;
      const old = wsRef.current;
      if (old) {
        old.onclose = null;
        old.close();
      }
      connect();
    });
    return () => {
      off();
      mountedRef.current = false;
      if (timerRef.current !== null) clearTimeout(timerRef.current);
      wsRef.current?.close();
    };
  }, [connect]);

  return state;
}
