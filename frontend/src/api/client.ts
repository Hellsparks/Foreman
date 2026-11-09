import type { HealthResponse } from "./types/HealthResponse";
import type { CreatePrinter } from "./types/CreatePrinter";
import type { Printer } from "./types/Printer";
import type { PrinterStatus } from "./types/PrinterStatus";

/** Base URL for all API calls.  Empty string = same origin (works in both dev
 *  with Vite proxy and in production served by foreman-server). */
const BASE = "";

class ApiError extends Error {
  constructor(
    public readonly status: number,
    message: string
  ) {
    super(message);
    this.name = "ApiError";
  }
}

async function get<T>(path: string): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    headers: { Accept: "application/json" },
  });
  if (!res.ok) {
    throw new ApiError(res.status, `GET ${path} → ${res.status}`);
  }
  return res.json() as Promise<T>;
}

async function post<T>(path: string, body?: unknown): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Accept: "application/json",
    },
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  if (!res.ok) {
    throw new ApiError(res.status, `POST ${path} → ${res.status}`);
  }
  return res.json() as Promise<T>;
}

async function put<T>(path: string, body?: unknown): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    method: "PUT",
    headers: {
      "Content-Type": "application/json",
      Accept: "application/json",
    },
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  if (!res.ok) {
    throw new ApiError(res.status, `PUT ${path} → ${res.status}`);
  }
  return res.json() as Promise<T>;
}

async function del(path: string): Promise<void> {
  const res = await fetch(`${BASE}${path}`, {
    method: "DELETE",
  });
  if (!res.ok) {
    throw new ApiError(res.status, `DELETE ${path} → ${res.status}`);
  }
}

/** A tiered connection path for a printer. */
export interface PrinterPath {
  id: number;
  printer_id: number;
  name: string;
  url: string;
  priority: number;
  created_at: string;
  updated_at: string;
}

/** Result of a single-path TCP reachability test. */
export interface PathTestResult {
  id: number;
  name: string;
  url: string;
  reachable: boolean;
  latency_ms: number | null;
}

/** Printer record enriched with live status from the server. */
export interface PrinterWithStatus {
  id: number;
  name: string;
  firmware: string;
  primary_url: string | null;
  api_key: string | null;
  serial_number: string | null;
  access_code: string | null;
  tags: string[];
  hardened_nozzle: boolean;
  sort_order: number;
  fail_after_checks: number;
  return_after_secs: number;
  created_at: string;
  updated_at: string;
  status: PrinterStatus | null;
  active_path_index?: number | null;
  active_path_name?: string | null;
}

/** An active attention item. */
export interface AttentionItem {
  id: number;
  printer_id: number | null;
  kind: string;
  title: string;
  body: string | null;
  created_at: string;
}

/** Temp preset for a printer. */
export interface TempPreset {
  id: number;
  printer_id: number;
  name: string;
  extruder_temp: number | null;
  bed_temp: number | null;
  chamber_temp: number | null;
  sort_order: number;
}

/** Macro favourite for a printer. */
export interface MacroFavourite {
  id: number;
  printer_id: number;
  macro_name: string;
}

/** Printer capabilities from the driver. */
export interface PrinterCapabilities {
  bed_size: [number, number] | null;
  extruder_count: number;
  webcams: { name: string; stream_url: string | null; snapshot_url: string | null }[];
  macros: string[];
  firmware_version: string | null;
  hostname: string | null;
  nozzle_diameter: number | null;
  ams_slot_count: number | null;
  camera_type: string | null;
  printer_model: string | null;
}

/** POST body that doesn't return JSON (204 No Content). */
async function postNoContent(path: string, body?: unknown): Promise<void> {
  const res = await fetch(`${BASE}${path}`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Accept: "application/json",
    },
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new ApiError(res.status, text || `POST ${path} → ${res.status}`);
  }
}

export const api = {
  health: () => get<HealthResponse>("/api/health"),
  listPrinters: () => get<PrinterWithStatus[]>("/api/printers"),
  createPrinter: (req: CreatePrinter) =>
    post<Printer>("/api/printers", req),
  deletePrinter: (id: number) => del(`/api/printers/${id}`),
  /** Update printer fields (name, fail_after_checks, return_after_secs, etc.). */
  updatePrinter: (
    id: number,
    body: Partial<{
      name: string;
      fail_after_checks: number;
      return_after_secs: number;
      hardened_nozzle: boolean;
      tags: string[];
      sort_order: number;
      primary_url: string | null;
      api_key: string | null;
    }>,
  ) => put<PrinterWithStatus>(`/api/printers/${id}`, body),
  /** URL for the camera snapshot proxy — use as <img src>. */
  snapshotUrl: (id: number) => `/api/printers/${id}/snapshot`,
  /** List active (uncleared) attention items. */
  listAttention: () => get<AttentionItem[]>("/api/attention"),
  /** Dismiss a single attention item. */
  dismissAttention: (id: number) =>
    post<void>(`/api/attention/${id}/dismiss`),
  /** Get live status for a single printer. */
  getStatus: (id: number) => get<PrinterStatus>(`/api/printers/${id}/status`),
  /** Get printer capabilities from the driver. */
  getCapabilities: (id: number) =>
    get<PrinterCapabilities>(`/api/printers/${id}/capabilities`),
  /** Send a command to a printer. */
  sendCommand: (id: number, kind: string, body: Record<string, unknown> = {}) =>
    postNoContent(`/api/printers/${id}/commands/${kind}`, body),
  /** Request a cancel confirm token. */
  confirmCancel: (id: number) =>
    post<{ confirm_token: string }>(`/api/printers/${id}/confirm-cancel`),
  /** List temp presets for a printer. */
  listPresets: (id: number) =>
    get<TempPreset[]>(`/api/printers/${id}/presets`),
  /** Create a temp preset. */
  createPreset: (id: number, body: { name: string; extruder_temp?: number; bed_temp?: number }) =>
    post<TempPreset>(`/api/printers/${id}/presets`, body),
  /** Delete a temp preset. */
  deletePreset: (printerId: number, presetId: number) =>
    del(`/api/printers/${printerId}/presets/${presetId}`),
  /** List macro favourites for a printer. */
  listFavourites: (id: number) =>
    get<MacroFavourite[]>(`/api/printers/${id}/macro-favourites`),
  /** Add a macro to favourites. */
  addFavourite: (id: number, macroName: string) =>
    post<MacroFavourite>(`/api/printers/${id}/macro-favourites`, { macro_name: macroName }),
  /** Remove a macro favourite. */
  removeFavourite: (printerId: number, favId: number) =>
    del(`/api/printers/${printerId}/macro-favourites/${favId}`),
  // ── Path management ───────────────────────────────────────────────────
  /** List connection paths for a printer (priority order). */
  listPaths: (id: number) => get<PrinterPath[]>(`/api/printers/${id}/paths`),
  /** Create a new connection path. */
  createPath: (id: number, body: { name: string; url: string; priority?: number }) =>
    post<PrinterPath>(`/api/printers/${id}/paths`, body),
  /** Update an existing connection path. */
  updatePath: (
    printerId: number,
    pathId: number,
    body: { name?: string; url?: string; priority?: number },
  ) => put<PrinterPath>(`/api/printers/${printerId}/paths/${pathId}`, body),
  /** Delete a connection path. */
  deletePath: (printerId: number, pathId: number) =>
    del(`/api/printers/${printerId}/paths/${pathId}`),
  /** Bulk-update path priorities (reorder). */
  reorderPaths: (id: number, items: { id: number; priority: number }[]) =>
    post<PrinterPath[]>(`/api/printers/${id}/paths/reorder`, items),
  /** TCP-ping all paths for a printer. */
  testAllPaths: (id: number) =>
    post<PathTestResult[]>(`/api/printers/${id}/paths/test-all`),
  /** Get the currently active path index + record. */
  getActivePath: (id: number) =>
    get<{ index: number; path: PrinterPath }>(`/api/printers/${id}/active-path`),
  /** Manually switch back to path 0 (only if path 0 is healthy). */
  switchBackToPath0: (id: number) =>
    postNoContent(`/api/printers/${id}/paths/switch-back`),
};

export { ApiError };
