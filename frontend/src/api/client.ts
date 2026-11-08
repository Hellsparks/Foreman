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

async function post<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Accept: "application/json",
    },
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    throw new ApiError(res.status, `POST ${path} → ${res.status}`);
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
  created_at: string;
  updated_at: string;
  status: PrinterStatus | null;
}

export const api = {
  health: () => get<HealthResponse>("/api/health"),
  listPrinters: () => get<PrinterWithStatus[]>("/api/printers"),
  createPrinter: (req: CreatePrinter) =>
    post<Printer>("/api/printers", req),
  deletePrinter: (id: number) => del(`/api/printers/${id}`),
};

export { ApiError };
