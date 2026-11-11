/** Queues + scheduler API (task 3.5): the Jobs → Queue board. */
import { request } from "./client";
import type { QueueBoard } from "./types/QueueBoard";
import type { QueueCard } from "./types/QueueCard";
import type { QueueItem } from "./types/QueueItem";
import type { QueueLane } from "./types/QueueLane";
import type { QueuePositions } from "./types/QueuePositions";
import type { StartNextResult } from "./types/StartNextResult";

export type { QueueBoard, QueueCard, QueueItem, QueueLane, QueuePositions, StartNextResult };

export const queue = {
  board: () => request<QueueBoard>("GET", "/api/queue"),
  /** `printer_id` null = fleet queue. */
  add: (file_id: number, printer_id: number | null, accept_manual_swaps = false) =>
    request<QueuePositions>("POST", "/api/queue", { file_id, printer_id, accept_manual_swaps }),
  move: (id: number, printer_id: number | null, index: number) =>
    request<QueuePositions>("POST", `/api/queue/${id}/move`, { printer_id, index }),
  remove: (id: number) => request<{ removed: "deleted" | "cancelled" }>("DELETE", `/api/queue/${id}`),
  startNext: (printerId: number, body: { plate_confirm?: boolean; dry_run?: boolean }) =>
    request<StartNextResult>("POST", `/api/printers/${printerId}/queue/start-next`, body),
  plateCleared: (printerId: number) => request<void>("POST", `/api/printers/${printerId}/plate-cleared`),
};
