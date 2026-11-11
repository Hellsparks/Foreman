/** Loaded spools + load spool flow (task 4.1). */
import { request } from "./client";
import type { EditSpool } from "./types/EditSpool";
import type { GuardResult } from "./types/GuardResult";
import type { GuardWarning } from "./types/GuardWarning";
import type { LoadCandidate } from "./types/LoadCandidate";
import type { LoadedAt } from "./types/LoadedAt";
import type { LoadedSlot } from "./types/LoadedSlot";
import type { LoadResult } from "./types/LoadResult";
import type { SpoolFacts } from "./types/SpoolFacts";
import type { SpoolRow } from "./types/SpoolRow";

export type { EditSpool, GuardResult, GuardWarning, LoadCandidate, LoadedAt, LoadedSlot, LoadResult, SpoolFacts, SpoolRow };

/** WS topic for `spool_loaded` events. */
export const SPOOL_TOPICS = ["spool"];

export const spools = {
  list: (archived = false) => request<SpoolRow[]>("GET", `/api/spools${archived ? "?archived=true" : ""}`),
  loaded: () => request<LoadedSlot[]>("GET", "/api/spools/loaded"),
  slots: (printerId: number) => request<LoadedSlot[]>("GET", `/api/printers/${printerId}/spools`),
  candidates: (printerId: number, tool: number) =>
    request<LoadCandidate[]>("GET", `/api/printers/${printerId}/spools/candidates?tool=${tool}`),
  check: (printerId: number, tool: number, spool_id: number) =>
    request<GuardResult>("POST", `/api/printers/${printerId}/spools/check`, { tool, spool_id }),
  load: (printerId: number, tool: number, spool_id: number, override_reason?: string) =>
    request<LoadResult>("POST", `/api/printers/${printerId}/spools/load`, {
      tool,
      spool_id,
      override_reason: override_reason ?? null,
    }),
  unload: (printerId: number, tool: number) =>
    request<LoadedSlot>("POST", `/api/printers/${printerId}/spools/unload`, { tool }),
  edit: (id: number, body: EditSpool) => request<SpoolFacts>("PATCH", `/api/spools/${id}`, body),
};

/** Low under this many grams (matches the server's guard floor). */
export const LOW_GRAMS = 150;

/** "PLA" / "PLA CF". */
export function materialLabel(s: SpoolFacts): string {
  return [s.material, s.modifier].filter(Boolean).join(" ") || "Unknown material";
}

/** "Single colour" / "Coextruded · 2 colours side by side" / "Gradient · 5 colours along the spool". */
export function colourKind(s: SpoolFacts): string {
  const n = s.multi_color_hexes.length;
  if (n < 2) return "Single colour";
  return s.multi_color_direction === "coaxial"
    ? `Coextruded · ${n} colours side by side`
    : `Gradient · ${n} colours along the spool`;
}

/** Fraction 0–1 of the spool left. */
export function leftFraction(s: SpoolFacts): number {
  if (s.remaining_weight == null || !s.initial_weight) return 0;
  return Math.max(0, Math.min(1, s.remaining_weight / s.initial_weight));
}

export function grams(g: number | null | undefined): string {
  return g == null ? "—" : `${Math.round(g)} g`;
}

/** "Loaded · HellV0 T0" / location / "—". */
export function whereLabel(s: SpoolFacts, loaded: LoadedAt[]): string {
  if (loaded.length) return `Loaded · ${loaded.map((l) => `${l.printer_name} ${l.label}`).join(", ")}`;
  return s.location || "—";
}
