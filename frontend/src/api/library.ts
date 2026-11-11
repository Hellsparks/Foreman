/** Library browser API: Library store, printer G-code storage, transfers (task 3.2), send (task 3.4). */
import { request } from "./client";
import { apiUrl, authHeaders } from "./conn";
import type { CompatResult } from "./types/CompatResult";
import type { LibraryEntry } from "./types/LibraryEntry";
import type { LibraryFileDetails } from "./types/LibraryFileDetails";
import type { SendResult } from "./types/SendResult";
import type { StorageEntry } from "./types/StorageEntry";
import type { TransferInfo } from "./types/TransferInfo";

export type { CompatResult, LibraryEntry, LibraryFileDetails, SendResult, StorageEntry, TransferInfo };

export type LocRef =
  | { kind: "library"; path: string }
  | { kind: "printer"; printer_id: number; path: string };

export type OnConflict = "ask" | "keep_both" | "replace" | "skip";

export interface PrinterLocation {
  printer_id: number;
  name: string;
  browsable: boolean;
  state: string | null;
  printing_file: string | null;
}

export interface ConflictInfo {
  index: number;
  name: string;
  is_dir: boolean;
  dest_printing: boolean;
}

export interface Blocked {
  name: string;
  reason: string;
}

export type StartOutcome =
  | { outcome: "conflicts"; conflicts: ConflictInfo[] }
  | { outcome: "started"; transfers: TransferInfo[]; skipped: string[]; blocked: Blocked[] };

export interface TransferBody {
  items: { src: LocRef; on_conflict?: OnConflict }[];
  dest: LocRef;
  mode: "copy" | "move";
  on_conflict: OnConflict;
}

const q = (path: string) => `path=${encodeURIComponent(path)}`;

function form(files: File[]): FormData {
  const f = new FormData();
  files.forEach((file) => f.append("file", file, file.name));
  return f;
}

export const lib = {
  list: (path: string) =>
    request<{ path: string; entries: LibraryEntry[] }>("GET", `/api/library?${q(path)}`),
  trash: () =>
    request<{ entries: LibraryEntry[]; purge_after_days: number }>("GET", "/api/library/trash"),
  details: (id: number) => request<LibraryFileDetails>("GET", `/api/library/files/${id}`),
  downloadPath: (id: number) => `/api/library/files/${id}/download`,
  upload: (path: string, files: File[]) =>
    request<{ entries: LibraryEntry[] }>("POST", `/api/library/upload?${q(path)}`, undefined, form(files)),
  mkdir: (path: string, name: string) =>
    request<LibraryEntry>("POST", "/api/library/folders", { path, name }),
  patch: (id: number, body: { name?: string; starred?: boolean }) =>
    request<LibraryEntry>("PATCH", `/api/library/entries/${id}`, body),
  trashMany: (ids: number[]) =>
    request<{ trashed: number[]; errors: { id: number; error: string }[] }>(
      "POST",
      "/api/library/trash-many",
      { ids },
    ),
  restoreMany: (ids: number[]) =>
    request<{ restored: LibraryEntry[]; errors: { id: number; error: string }[] }>(
      "POST",
      "/api/library/restore-many",
      { ids },
    ),
};

export const storage = {
  locations: () => request<{ printers: PrinterLocation[] }>("GET", "/api/locations"),
  list: (id: number, path: string) =>
    request<{ printer_id: number; name: string; path: string; entries: StorageEntry[]; active_file: string | null }>(
      "GET",
      `/api/printers/${id}/storage?${q(path)}`,
    ),
  thumbPath: (id: number, path: string) => `/api/printers/${id}/storage/thumb?${q(path)}`,
  downloadPath: (id: number, path: string) => `/api/printers/${id}/storage/download?${q(path)}`,
  mkdir: (id: number, path: string, name: string) =>
    request<{ path: string }>("POST", `/api/printers/${id}/storage/folders`, { path, name }),
  rename: (id: number, path: string, name: string) =>
    request<{ path: string }>("POST", `/api/printers/${id}/storage/rename`, { path, name }),
  remove: (id: number, paths: string[]) =>
    request<{ deleted: string[]; blocked: Blocked[] }>("POST", `/api/printers/${id}/storage/delete`, { paths }),
  upload: (id: number, path: string, files: File[]) =>
    request<{ paths: string[] }>("POST", `/api/printers/${id}/storage/upload?${q(path)}`, undefined, form(files)),
};

export const transfers = {
  list: () => request<{ transfers: TransferInfo[] }>("GET", "/api/transfers"),
  cancel: (id: string) => request<TransferInfo>("POST", `/api/transfers/${id}/cancel`),
  clear: () => request<void>("POST", "/api/transfers/clear"),
  /** 409 carries the conflicts to resolve, so it is a result here, not an error. */
  start: async (body: TransferBody): Promise<StartOutcome> => {
    const res = await fetch(apiUrl("/api/transfers"), {
      method: "POST",
      headers: { "Content-Type": "application/json", Accept: "application/json", ...authHeaders() },
      body: JSON.stringify(body),
    });
    const json = await res.json().catch(() => null);
    if (res.status === 409 || res.ok) return json as StartOutcome;
    throw new Error((json && typeof json.error === "string" && json.error) || `transfer → ${res.status}`);
  },
};

export const send = {
  /** Compatibility check: returns per-printer verdicts for a library file. */
  check: (fileId: number) => request<CompatResult[]>("GET", `/api/send/check/${fileId}`),
  /** Send a file to a printer (print_now, queue, fleet_queue). */
  send: (body: { file_id: number; printer_id?: number; mode: string; plate_confirm?: boolean }) =>
    request<SendResult>("POST", "/api/send", body),
};
