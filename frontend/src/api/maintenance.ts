/** Maintenance (task 5.3, W12). */
import { request } from "./client";
import type { Basis } from "./types/Basis";
import type { Counters } from "./types/Counters";
import type { DueState } from "./types/DueState";
import type { MaintenanceTask } from "./types/MaintenanceTask";
import type { Overview } from "./types/Overview";
import type { Progress } from "./types/Progress";
import type { ServiceLog } from "./types/ServiceLog";
import type { TaskForm } from "./types/TaskForm";
import type { TaskTemplate } from "./types/TaskTemplate";
import type { TaskView } from "./types/TaskView";

export type { Basis, Counters, DueState, MaintenanceTask, Overview, Progress, ServiceLog, TaskForm, TaskTemplate, TaskView };

/** Attention kinds raised by maintenance are `maintenance:<task id>`. */
export const ATTENTION_PREFIX = "maintenance:";
export const MAINTENANCE_TOPICS = ["job", "attention.added", "attention.updated", "attention.cleared"];

export const maintenanceApi = {
  overview: (printerId?: number) =>
    request<Overview>("GET", `/api/maintenance${printerId != null ? `?printer_id=${printerId}` : ""}`),
  setDueSoon: (pct: number) => request<{ due_soon_pct: number }>("PUT", "/api/maintenance/settings", { due_soon_pct: pct }),
  templates: () => request<TaskTemplate[]>("GET", "/api/maintenance/templates"),
  fromTemplates: (printerId: number, keys: string[], startNow: boolean) =>
    request<TaskView[]>("POST", "/api/maintenance/from-templates", { printer_id: printerId, keys, start_now: startNow }),
  create: (f: TaskForm) => request<TaskView>("POST", "/api/maintenance/tasks", f),
  update: (id: number, f: TaskForm) => request<TaskView>("PUT", `/api/maintenance/tasks/${id}`, f),
  remove: (id: number) => request<void>("DELETE", `/api/maintenance/tasks/${id}`),
  done: (id: number, notes: string) =>
    request<{ task: TaskView; log: ServiceLog }>("POST", `/api/maintenance/tasks/${id}/done`, { notes }),
  snooze: (id: number, snooze?: number) =>
    request<TaskView>("POST", `/api/maintenance/tasks/${id}/snooze`, snooze == null ? {} : { snooze }),
  history: (q: { taskId?: number; printerId?: number }) => {
    const p = new URLSearchParams();
    if (q.taskId != null) p.set("task_id", String(q.taskId));
    if (q.printerId != null) p.set("printer_id", String(q.printerId));
    return request<ServiceLog[]>("GET", `/api/maintenance/history?${p}`);
  },
  undoDone: (logId: number) => request<void>("DELETE", `/api/maintenance/history/${logId}`),
};

/** Whole numbers without decimals; small values keep two. */
export function num(v: number): string {
  if (Math.abs(v - Math.round(v)) < 0.005) return String(Math.round(v));
  return Math.abs(v) < 10 ? v.toFixed(2) : v.toFixed(1);
}

/** Amount with its unit: "212 h", "1.2 kg", "14 jobs". */
export function amount(v: number, basis: Basis): string {
  if (basis === "hours") return `${num(v)} h`;
  if (basis === "grams") return v >= 1000 ? `${num(v / 1000)} kg` : `${num(v)} g`;
  return `${num(v)} job${Math.round(v) === 1 ? "" : "s"}`;
}

export function unitLong(basis: Basis): string {
  return basis === "hours" ? "print hours" : basis === "grams" ? "g filament" : "jobs";
}

/** "every 200 print hours · or 90 days". */
export function everyText(t: Pick<MaintenanceTask, "basis" | "interval" | "every_days">): string {
  const main = t.basis === "grams" && t.interval >= 1000 ? `${num(t.interval / 1000)} kg filament` : `${num(t.interval)} ${unitLong(t.basis)}`;
  return t.every_days ? `${main} · or ${t.every_days} days` : main;
}

/** "✖ Overdue · 212 / 200 h", "⚠ Due soon · …", "✓ 41 / 50 h". */
export function statusText(v: TaskView): string {
  const p = v.progress;
  const sym = p.state === "overdue" ? "✖ Overdue · " : p.state === "soon" ? "⚠ Due soon · " : "✓ ";
  const main =
    p.by_days && p.days_limit != null
      ? `${num(p.days)} / ${num(p.days_limit)} days`
      : `${num(p.value)} / ${amount(p.limit, v.task.basis)}`;
  const snoozed = v.task.snooze > 0 ? " · snoozed" : "";
  return sym + main + snoozed;
}

export function tone(s: DueState): "bad" | "warn" | "ok" {
  return s === "overdue" ? "bad" : s === "soon" ? "warn" : "ok";
}

/** "12 Nov" (this year) or "12 Nov 2025". */
export function shortDate(iso: string): string {
  const d = new Date(iso);
  const sameYear = d.getFullYear() === new Date().getFullYear();
  return d.toLocaleDateString(undefined, sameYear ? { day: "numeric", month: "short" } : { day: "numeric", month: "short", year: "numeric" });
}

/** Gap since the previous service: "3.5 h · 41 days after previous". */
export function gapText(l: ServiceLog): string {
  const days = l.days < 1 ? "< 1 day" : `${num(Math.round(l.days))} day${Math.round(l.days) === 1 ? "" : "s"}`;
  const from = l.first ? "since counting started" : "after previous";
  return `${amount(l.counter, l.basis)} · ${days} ${from}`;
}
