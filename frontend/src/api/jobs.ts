/** Job history + stats API (task 3.6). */
import { request } from "./client";
import type { Job } from "./types/Job";
import type { JobList } from "./types/JobList";
import type { JobOutcome } from "./types/JobOutcome";
import type { JobStats } from "./types/JobStats";

export type { Job, JobList, JobOutcome, JobStats };

export interface HistoryFilter {
  printer_id?: number | null;
  from?: string;
  to?: string;
  material?: string;
  spool_id?: number | null;
  /** good | failed | cancelled | unrated | running | all */
  outcome?: string;
  q?: string;
  limit?: number;
}

function qs(params: Record<string, string | number | null | undefined>): string {
  const p = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined && v !== null && v !== "") p.set(k, String(v));
  }
  const s = p.toString();
  return s ? `?${s}` : "";
}

export const FAIL_REASONS = ["adhesion", "spaghetti", "layer shift", "clog", "warping", "other"];

export const jobs = {
  list: (f: HistoryFilter = {}) => request<JobList>("GET", `/api/jobs${qs({ ...f })}`),
  get: (id: number) => request<Job>("GET", `/api/jobs/${id}`),
  rate: (id: number, outcome: JobOutcome, fail_reason?: string | null) =>
    request<Job>("PATCH", `/api/jobs/${id}`, { outcome, fail_reason: fail_reason ?? null }),
  remove: (id: number) => request<void>("DELETE", `/api/jobs/${id}`),
  stats: (from?: string, to?: string) => request<JobStats>("GET", `/api/jobs/stats${qs({ from, to })}`),
};
