/** Templates and projects (task 5.1, W9). */
import { request } from "./client";
import type { CreateProject } from "./types/CreateProject";
import type { LivePrint } from "./types/LivePrint";
import type { PlateCounts } from "./types/PlateCounts";
import type { PlateFile } from "./types/PlateFile";
import type { ProjectPlan } from "./types/ProjectPlan";
import type { ProjectPlate } from "./types/ProjectPlate";
import type { ProjectSlot } from "./types/ProjectSlot";
import type { ProjectSummary } from "./types/ProjectSummary";
import type { ProjectTotals } from "./types/ProjectTotals";
import type { ProjectView } from "./types/ProjectView";
import type { SlotPlan } from "./types/SlotPlan";
import type { SpoolOption } from "./types/SpoolOption";
import type { TemplateForm } from "./types/TemplateForm";
import type { TemplatePlate } from "./types/TemplatePlate";
import type { TemplateSlot } from "./types/TemplateSlot";
import type { TemplateView } from "./types/TemplateView";
import type { ToolSlot } from "./types/ToolSlot";
import type { UpdateProject } from "./types/UpdateProject";

export type {
  CreateProject,
  LivePrint,
  PlateCounts,
  PlateFile,
  ProjectPlan,
  ProjectPlate,
  ProjectSlot,
  ProjectSummary,
  ProjectTotals,
  ProjectView,
  SlotPlan,
  SpoolOption,
  TemplateForm,
  TemplatePlate,
  TemplateSlot,
  TemplateView,
  ToolSlot,
  UpdateProject,
};

export const MAX_COPIES = 50;
export const MAX_SETS = 50;

export const projectsApi = {
  templates: () => request<TemplateView[]>("GET", "/api/templates"),
  template: (id: number) => request<TemplateView>("GET", `/api/templates/${id}`),
  createTemplate: (f: TemplateForm) => request<TemplateView>("POST", "/api/templates", f),
  saveTemplate: (id: number, f: TemplateForm) => request<TemplateView>("PUT", `/api/templates/${id}`, f),
  deleteTemplate: (id: number) => request<void>("DELETE", `/api/templates/${id}`),
  plan: (id: number, sets: number) => request<ProjectPlan>("GET", `/api/templates/${id}/plan?sets=${sets}`),
  list: () => request<ProjectSummary[]>("GET", "/api/projects"),
  get: (id: number) => request<ProjectView>("GET", `/api/projects/${id}`),
  create: (b: CreateProject) => request<ProjectView>("POST", "/api/projects", b),
  update: (id: number, b: UpdateProject) => request<ProjectView>("PATCH", `/api/projects/${id}`, b),
  printRemaining: (id: number) => request<{ added: number }>("POST", `/api/projects/${id}/print-remaining`),
  setSlot: (id: number, key: string, spool_id: number | null) =>
    request<ProjectView>("PUT", `/api/projects/${id}/slots/${encodeURIComponent(key)}`, { spool_id }),
  setCopies: (plateId: number, copies: number) =>
    request<ProjectView>("PATCH", `/api/project-plates/${plateId}`, { copies }),
  markDone: (plateId: number) => request<ProjectView>("POST", `/api/project-plates/${plateId}/done`),
};

/** "PRIMARY" style key. */
export function slotKey(s: string): string {
  return s.trim().split(/\s+/).join(" ").toUpperCase();
}

/** Which slot a plate's tool uses, or null. */
export function slotOf(p: TemplatePlate, tool: number): string | null {
  return p.tools.find((t) => t.tool === tool)?.slot ?? null;
}

/**
 * Tick/untick slot `key` on a plate. A plate with one tool (or none parsed)
 * maps T0; with several tools, `tool` says which one.
 */
export function setPlateSlot(p: TemplatePlate, tool: number, key: string | null): TemplatePlate {
  const rest = p.tools.filter((t) => t.tool !== tool);
  const tools = key ? [...rest, { tool, slot: slotKey(key) }] : rest;
  return { ...p, tools: tools.sort((a, b) => a.tool - b.tool) };
}

/** Rename a slot everywhere it is used. */
export function renameSlot(f: TemplateForm, from: string, to: string): TemplateForm {
  const k = slotKey(to);
  return {
    ...f,
    slots: f.slots.map((s) => (s.key === from ? { ...s, key: k } : s)),
    plates: f.plates.map((p) => ({ ...p, tools: p.tools.map((t) => (t.slot === from ? { ...t, slot: k } : t)) })),
  };
}

/** A free slot name: PRIMARY, ACCENT, then SLOT 3, … */
export function nextSlotKey(f: TemplateForm): string {
  const used = new Set(f.slots.map((s) => s.key));
  for (const k of ["PRIMARY", "ACCENT", "DETAIL", "TRIM"]) if (!used.has(k)) return k;
  for (let i = f.slots.length + 1; ; i++) if (!used.has(`SLOT ${i}`)) return `SLOT ${i}`;
}

export function printsPerSet(f: TemplateForm): number {
  return f.plates.reduce((a, p) => a + p.copies, 0);
}

/** "3 of 8 prints done · 1 printing · 2 queued · 2 not started". */
export function progressText(t: ProjectTotals): string {
  const parts = [`${t.printed} of ${t.prints} print${t.prints === 1 ? "" : "s"} done`];
  if (t.printing) parts.push(`${t.printing} printing`);
  if (t.queued) parts.push(`${t.queued} queued`);
  if (t.remaining) parts.push(`${t.remaining} not started`);
  return parts.join(" · ");
}

/** Plate status line from its counts and live prints. */
export function plateStatus(c: PlateCounts, live: LivePrint[]): { tone: "ok" | "info" | "warn" | "off" | "bad"; text: string } {
  const failed = live.filter((l) => l.status === "failed").length;
  const printing = live.filter((l) => l.status === "printing" || l.status === "sending");
  if (c.printed >= c.copies && !printing.length && !c.queued)
    return { tone: "ok", text: c.copies > 1 ? `✓ Done ×${c.copies}` : "✓ Done" };
  if (printing.length) {
    const where = printing.map((l) => l.where_).join(", ");
    const copy = c.copies > 1 ? ` · copy ${Math.min(c.copies, c.printed + 1)}/${c.copies}` : "";
    return { tone: "info", text: `▶ Printing${copy} · ${where}` };
  }
  if (failed) return { tone: "bad", text: `✖ ${failed} failed on the queue` };
  if (c.queued) {
    const where = [...new Set(live.filter((l) => l.status === "waiting").map((l) => l.where_))].join(", ") || "fleet";
    return { tone: "off", text: c.printed ? `◐ ${c.printed} of ${c.copies} done · ${c.queued} queued · ${where}` : `◷ Queued ×${c.queued} · ${where}` };
  }
  if (c.printed) return { tone: "warn", text: `◐ ${c.printed} of ${c.copies} done` };
  return { tone: "warn", text: "○ Not started" };
}

export function hours(secs: number | null | undefined): string {
  if (!secs) return "—";
  const h = Math.floor(secs / 3600);
  const m = Math.round((secs % 3600) / 60);
  return h ? `${h} h ${String(m).padStart(2, "0")} m` : `${m} m`;
}

/** Fit line for a spool option. */
export function fitText(o: SpoolOption, need: number): { tone: "ok" | "warn" | "bad"; text: string } {
  if (!o.fits) return { tone: "bad", text: `✖ short ${Math.round(need - o.free)} g of ${Math.round(Math.max(0, o.free))} g free` };
  if (!o.material_ok) return { tone: "warn", text: `⚠ fits · ${o.spool.material ?? "unknown material"}, plates want another` };
  if (o.colour_distance != null && o.colour_distance > 60) return { tone: "warn", text: `⚠ fits · colour differs · ${Math.round(o.free - need)} g spare` };
  return { tone: "ok", text: `✓ fits · ${Math.round(o.free - need)} g spare` };
}
