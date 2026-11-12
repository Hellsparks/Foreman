/** Pure helpers for the setup wizard and the empty-Fleet checklist (unit-tested). */
import type { Tone } from "../ui";
import type { FoundPrinter, ImportCounts, PendingImport, Readiness, SetupStep } from "../api/setup";

export const STEPS: { step: SetupStep; label: string }[] = [
  { step: "data", label: "Filament data" },
  { step: "printers", label: "Printers" },
  { step: "finish", label: "Finish" },
];

export function stepIndex(step: SetupStep): number {
  const i = STEPS.findIndex((s) => s.step === step);
  return i < 0 ? STEPS.length : i;
}

/** "Getting started · 2 of 5 done" */
export function checklistHeading(done: number, total: number): string {
  return done >= total ? "Getting started · all done" : `Getting started · ${done} of ${total} done`;
}

export function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

/** "90 spools · 288 filaments · 64 vendors" */
export function countsLine(c: ImportCounts): string {
  const parts = [
    plural(c.spools, "spool", "spools"),
    plural(c.filaments, "filament", "filaments"),
    plural(c.vendors, "vendor", "vendors"),
  ];
  if (c.archived_spools > 0) parts.push(`${c.archived_spools} archived`);
  return parts.join(" · ");
}

export function isEmpty(c: ImportCounts): boolean {
  return c.spools === 0 && c.filaments === 0 && c.vendors === 0;
}

/** What a detected revision means, in one line. */
export function revisionLine(p: PendingImport): string {
  const rev = p.revision ?? "none";
  switch (p.status) {
    case "head":
      return `Revision ${rev} · current, used as is`;
    case "older":
      return `Revision ${rev} · older, ${plural(p.steps_to_head, "migration step", "migration steps")} on import (on a copy)`;
    case "unknown":
      return `Revision ${rev} · unknown (newer than ${p.head})`;
    default:
      return "No alembic_version table";
  }
}

export interface Chip {
  tone: Tone;
  symbol: string;
  word: string;
  short: string;
}

export function readinessChip(p: Pick<FoundPrinter, "needs" | "added_as">): Chip {
  if (p.added_as != null) return { tone: "off", symbol: "✓", word: "Added", short: "Added" };
  return chipFor(p.needs);
}

export function chipFor(needs: Readiness): Chip {
  switch (needs) {
    case "ready":
      return { tone: "ok", symbol: "✓", word: "Ready", short: "Ready" };
    case "api_key":
      return { tone: "warn", symbol: "⚠", word: "Key needed", short: "Key" };
    case "access_code":
      return { tone: "warn", symbol: "⚠", word: "Access code", short: "Code" };
    default:
      return { tone: "bad", symbol: "○", word: "No answer", short: "Offline" };
  }
}

export function firmwareLabel(fw: string): string {
  return fw === "moonraker" ? "Moonraker" : fw === "bambu" ? "Bambu" : fw;
}

/** "10.0.0.177 · Moonraker v0.11.0 · Klipper" / "10.0.0.249 · A1 · serial 03919C461200718" */
export function foundSubline(p: FoundPrinter): string {
  const parts = [p.host];
  if (p.firmware === "moonraker") {
    parts.push(p.version ? `Moonraker ${p.version}` : "Moonraker");
    if (p.model) parts.push(p.model);
  } else {
    parts.push(p.model ? `Bambu ${p.model}` : "Bambu");
    if (p.serial) parts.push(`serial ${p.serial}`);
  }
  return parts.join(" · ");
}

export const validAccessCode = (s: string) => /^[A-Za-z0-9]{8}$/.test(s.trim());
export const validSerial = (s: string) => /^[0-9A-Z]{15}$/.test(s.trim());

export interface Creds {
  apiKey: string;
  accessCode: string;
  serial: string;
}

export const NO_CREDS: Creds = { apiKey: "", accessCode: "", serial: "" };

/** Why a picked printer can't be added yet (null = it can). */
export function missingFor(p: FoundPrinter, c: Creds): string | null {
  if (p.added_as != null) return "Already added";
  if (p.needs === "api_key" && !c.apiKey.trim()) return "Enter the Moonraker API key";
  if (p.firmware === "bambu") {
    if (!p.serial && !validSerial(c.serial)) return "Enter the 15-character serial";
    if (!validAccessCode(c.accessCode)) return "Enter the 8-character access code";
  }
  return null;
}

/** "Scanned 12 s ago" */
export function scannedAgo(iso: string, now: number): string {
  const s = Math.max(0, Math.round((now - Date.parse(iso)) / 1000));
  if (s < 60) return `Scanned ${s} s ago`;
  const m = Math.round(s / 60);
  return `Scanned ${m} min ago`;
}

/** "Add 2 printers" */
export function addLabel(n: number): string {
  return n === 0 ? "Add printers" : `Add ${plural(n, "printer", "printers")}`;
}
