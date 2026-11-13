import type { HealthResponse } from "./types/HealthResponse";
import type { CreatePrinter } from "./types/CreatePrinter";
import type { Printer } from "./types/Printer";
import type { PrinterStatus } from "./types/PrinterStatus";
import type { PrinterSsh } from "./types/PrinterSsh";
import type { UpsertPrinterSsh } from "./types/UpsertPrinterSsh";
import type { SftpEntry } from "./types/SftpEntry";
import type { ShellMacro } from "./types/ShellMacro";
import type { CreateShellMacro } from "./types/CreateShellMacro";
import type { UpdateShellMacro } from "./types/UpdateShellMacro";
import type { ApiKey } from "./types/ApiKey";
import { apiUrl, authHeaders, wsUrl } from "./conn";

export type { PrinterSsh, UpsertPrinterSsh, SftpEntry, ShellMacro, CreateShellMacro, UpdateShellMacro };

class ApiError extends Error {
  constructor(
    public readonly status: number,
    message: string,
    public readonly code: string | null = null,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

/** Fired when a protected call gets 401 so the app can show the sign-in screen. */
export const UNAUTHORIZED_EVENT = "foreman:unauthorized";

/** Fetch against the current server, with its bearer token (desktop app) if any. */
function serverFetch(path: string, init?: RequestInit): Promise<Response> {
  return fetch(apiUrl(path), {
    ...init,
    headers: { ...(init?.headers as Record<string, string> | undefined), ...authHeaders() },
  });
}

async function apiFetch(path: string, init?: RequestInit): Promise<Response> {
  const res = await serverFetch(path, init);
  if (res.status === 401) window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));
  return res;
}

async function get<T>(path: string): Promise<T> {
  const res = await apiFetch(path, {
    headers: { Accept: "application/json" },
  });
  if (!res.ok) {
    throw new ApiError(res.status, `GET ${path} → ${res.status}`);
  }
  return res.json() as Promise<T>;
}

async function post<T>(path: string, body?: unknown): Promise<T> {
  const res = await apiFetch(path, {
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
  const res = await apiFetch(path, {
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
  const res = await apiFetch(path, {
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
  /** Who reports filament use to Spoolman (auto | moonraker | foreman). */
  usage_reporter?: "auto" | "moonraker" | "foreman";
  created_at: string;
  updated_at: string;
  status: PrinterStatus | null;
  active_path_index?: number | null;
  active_path_name?: string | null;
  /** Set when Foreman can't send, start, pause, resume or cancel prints here. */
  control_unsupported?: string | null;
}

/** An active attention item. */
export interface AttentionItem {
  id: number;
  printer_id: number | null;
  kind: string;
  title: string;
  body: string | null;
  /** The job a "print finished" item is about (rate it Good / Failed). */
  job_id?: number | null;
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
  const res = await apiFetch(path, {
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

/** Fetch that surfaces the server's `{ error, code }` body in the thrown ApiError. */
async function req<T>(
  method: string,
  path: string,
  body?: unknown,
  raw?: BodyInit,
  signalUnauthorized = true,
): Promise<T> {
  const headers: Record<string, string> = { Accept: "application/json" };
  if (body !== undefined) headers["Content-Type"] = "application/json";
  const init: RequestInit = {
    method,
    headers,
    body: raw ?? (body !== undefined ? JSON.stringify(body) : undefined),
  };
  const res = signalUnauthorized ? await apiFetch(path, init) : await serverFetch(path, init);
  if (!res.ok) {
    let msg = `${method} ${path} → ${res.status}`;
    let code: string | null = null;
    try {
      const j = await res.json();
      if (j && typeof j.error === "string") msg = j.error;
      if (j && typeof j.code === "string") code = j.code;
    } catch {
      /* non-JSON error body */
    }
    throw new ApiError(res.status, msg, code);
  }
  if (res.status === 204) return undefined as T;
  return res.json() as Promise<T>;
}

/** POST JSON, get a file back (zip / STL / JSON download) with its suggested name. */
async function postBlob(path: string, body: unknown): Promise<{ blob: Blob; filename: string | null }> {
  const res = await apiFetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    let msg = `POST ${path} → ${res.status}`;
    try {
      const j = await res.json();
      if (j && typeof j.error === "string") msg = j.error;
    } catch {
      /* non-JSON error body */
    }
    throw new ApiError(res.status, msg);
  }
  const cd = res.headers.get("Content-Disposition") ?? "";
  const m = /filename="([^"]+)"/.exec(cd);
  return { blob: await res.blob(), filename: m ? m[1] : null };
}

/** GET a file (CSV / JSON download) with its suggested name. */
async function getBlob(path: string): Promise<{ blob: Blob; filename: string | null }> {
  const res = await apiFetch(path, { method: "GET" });
  if (!res.ok) {
    let msg = `GET ${path} → ${res.status}`;
    try {
      const j = await res.json();
      if (j && typeof j.error === "string") msg = j.error;
    } catch {
      /* non-JSON error body */
    }
    throw new ApiError(res.status, msg);
  }
  const cd = res.headers.get("Content-Disposition") ?? "";
  const m = /filename="([^"]+)"/.exec(cd);
  return { blob: await res.blob(), filename: m ? m[1] : null };
}

/** Hands a blob to the browser as a download. */
export function saveBlob(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

/** Quick roots on a Klipper host (absolute paths). */
export interface SshRoots {
  home: string;
  config: string;
  gcodes: string;
  logs: string;
}

export interface FilesInfo {
  user: string;
  host: string;
  port: number;
  roots: SshRoots;
}

export interface TextFile {
  path: string;
  content: string;
  size: number;
  truncated: boolean;
  read_only: boolean;
}

const q = (path: string) => `path=${encodeURIComponent(path)}`;

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
      access_code: string | null;
      usage_reporter: "auto" | "moonraker" | "foreman";
    }>,
  ) => put<PrinterWithStatus>(`/api/printers/${id}`, body),
  /** Camera snapshot path — turn into an <img src> with `useSignedUrl`; `t` busts the cache. */
  snapshotPath: (id: number, t?: number) => `/api/printers/${id}/snapshot${t ? `?t=${t}` : ""}`,
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
  // ── SSH / SFTP / shell-macros ──────────────────────────────────────────
  getSsh: (id: number) => req<PrinterSsh>("GET", `/api/printers/${id}/ssh`),
  upsertSsh: (id: number, body: UpsertPrinterSsh) =>
    req<PrinterSsh>("PUT", `/api/printers/${id}/ssh`, body),
  deleteSsh: (id: number) => req<void>("DELETE", `/api/printers/${id}/ssh`),
  /** Foreman's ed25519 public key (authorized_keys line). */
  getSshPublicKey: () => req<{ public_key: string }>("GET", "/api/ssh/public-key"),
  /** Pin the changed host key the printer presented. */
  trustHostKey: (id: number) => req<PrinterSsh>("POST", `/api/printers/${id}/ssh/trust`),
  /** Connect + authenticate; runs no command. */
  testSsh: (id: number) =>
    req<{ ok: boolean; user: string; host: string; home: string; host_key_fingerprint: string | null }>(
      "POST",
      `/api/printers/${id}/ssh/test`,
    ),
  filesInfo: (id: number) => req<FilesInfo>("GET", `/api/printers/${id}/files/roots`),
  /** Pooled SFTP session state on the server. */
  sshSession: (id: number) =>
    req<{ open: boolean; open_secs?: number; idle_secs?: number }>("GET", `/api/printers/${id}/ssh/session`),
  closeSshSession: (id: number) => req<void>("DELETE", `/api/printers/${id}/ssh/session`),
  listFiles: (id: number, path: string) =>
    req<SftpEntry[]>("GET", `/api/printers/${id}/files?${q(path)}`),
  readFile: (id: number, path: string) =>
    req<TextFile>("GET", `/api/printers/${id}/files/read?${q(path)}`),
  downloadPath: (id: number, path: string) => `/api/printers/${id}/files/download?${q(path)}`,
  /** Inline image path for previews (raster formats only). */
  previewPath: (id: number, path: string) =>
    `/api/printers/${id}/files/download?${q(path)}&inline=true`,
  /** Whole-file write; the server keeps `<file>.bak`. */
  writeFile: (id: number, path: string, content: string) =>
    req<{ ok: boolean; backup: boolean }>("PUT", `/api/printers/${id}/files/write`, { path, content }),
  /** Save a Klipper .cfg then FIRMWARE_RESTART (refused while printing). */
  saveAndRestart: (id: number, path: string, content: string) =>
    req<{ ok: boolean; backup: boolean; restarted: boolean }>(
      "POST",
      `/api/printers/${id}/files/save-restart`,
      { path, content },
    ),
  uploadFile: (id: number, path: string, file: Blob) =>
    req<{ ok: boolean; path: string; size: number }>(
      "POST",
      `/api/printers/${id}/files/upload?${q(path)}`,
      undefined,
      file,
    ),
  deleteFile: (id: number, path: string) =>
    req<void>("DELETE", `/api/printers/${id}/files?${q(path)}`),
  mkdir: (id: number, path: string) =>
    req<{ ok: boolean; path: string }>("POST", `/api/printers/${id}/files/mkdir`, { path }),
  renameFile: (id: number, from: string, to: string) =>
    req<{ ok: boolean; path: string }>("POST", `/api/printers/${id}/files/rename`, { from, to }),
  listShellMacros: (id: number) =>
    req<ShellMacro[]>("GET", `/api/printers/${id}/shell-macros`),
  createShellMacro: (id: number, body: CreateShellMacro) =>
    req<ShellMacro>("POST", `/api/printers/${id}/shell-macros`, body),
  updateShellMacro: (id: number, mid: number, body: UpdateShellMacro) =>
    req<ShellMacro>("PUT", `/api/printers/${id}/shell-macros/${mid}`, body),
  deleteShellMacro: (id: number, mid: number) =>
    req<void>("DELETE", `/api/printers/${id}/shell-macros/${mid}`),
  /** WebSocket URL for the SSH terminal. */
  terminalWsUrl: (id: number, cols: number, rows: number): Promise<string> =>
    wsUrl(`/ws/printers/${id}/terminal?cols=${cols}&rows=${rows}`),
  // ── Auth ─────────────────────────────────────────────────────────────────
  serverInfo: () => req<ServerInfo>("GET", "/api/info", undefined, undefined, false),
  me: () => req<{ username: string; kind: string }>("GET", "/api/auth/me", undefined, undefined, false),
  setupAdmin: (body: { code: string; username: string; password: string; remember: boolean; return_token?: boolean }) =>
    req<SessionResponse>("POST", "/api/auth/setup", body, undefined, false),
  login: (body: { username: string; password: string; remember: boolean; return_token?: boolean }) =>
    req<SessionResponse>("POST", "/api/auth/login", body, undefined, false),
  logout: () => req<void>("POST", "/api/auth/logout", undefined, undefined, false),
  listApiKeys: () => req<ApiKey[]>("GET", "/api/auth/api-keys"),
  /** `secret` is only returned here, once. */
  createApiKey: (name: string) =>
    req<{ key: ApiKey; secret: string }>("POST", "/api/auth/api-keys", { name }),
  revokeApiKey: (id: number) => req<void>("DELETE", `/api/auth/api-keys/${id}`),
  getNetwork: () => req<NetworkSettings>("GET", "/api/settings/network"),
  setNetwork: (allow_lan: boolean) =>
    req<NetworkSettings>("PUT", "/api/settings/network", { allow_lan }),
  renameServer: (name: string) =>
    req<{ server_id: string; name: string }>("PUT", "/api/settings/server", { name }),
};

/** `token` only when `return_token` was asked for (desktop app). */
export interface SessionResponse {
  username: string;
  expires_at: number;
  token?: string;
}

export interface ReportedAddress {
  address: string;
  kind: "loopback" | "lan_ip" | "lan_name" | "tailscale_ip" | "tailscale_name";
}

export interface ServerInfo {
  server_id: string;
  name: string;
  version: string;
  needs_setup: boolean;
  addresses: ReportedAddress[];
}

export interface NetworkSettings {
  desktop_mode: boolean;
  allow_lan: boolean;
  configured_host: string;
  bound_host: string;
  restart_required: boolean;
}

export type { ApiKey };

export { ApiError, getBlob, postBlob, req as request };
