/** Pure helpers for Settings → Backup. */
import type { BackupCounts } from "../api/types/BackupCounts";
import type { BackupSettingsDto } from "../api/types/BackupSettingsDto";
import type { InspectResult } from "../api/types/InspectResult";

export function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v >= 100 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
}

export const WEEKDAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"] as const;

/** "Daily at 03:00" / "Weekly on Sunday at 03:00" / "Off". */
export function scheduleText(s: Pick<BackupSettingsDto, "schedule" | "time" | "weekday">): string {
  if (s.schedule === "off") return "Off";
  if (s.schedule === "daily") return `Daily at ${s.time}`;
  return `Weekly on ${WEEKDAYS[(s.weekday - 1 + 7) % 7]} at ${s.time}`;
}

export function fmtWhen(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
}

const COUNT_LABELS: [keyof BackupCounts, string][] = [
  ["printers", "Printers"],
  ["spools", "Spools"],
  ["filaments", "Filaments"],
  ["vendors", "Vendors"],
  ["jobs", "Jobs"],
  ["projects", "Projects"],
  ["library_files", "Library files (indexed)"],
];

/** Rows for the restore check: what the backup has vs what is live now. */
export function compareRows(r: InspectResult): { label: string; backup: string; now: string; differs: boolean }[] {
  const m = r.manifest;
  const rows = [
    { label: "Taken", backup: fmtWhen(m.created_at), now: "now" },
    { label: "Foreman", backup: `${m.foreman_version} · schema ${m.foreman_schema}`, now: `${r.current.foreman_version} · schema ${r.current.foreman_schema}` },
    { label: "Spoolman schema", backup: m.spoolman_revision ?? "not included", now: r.current.spoolman_revision },
    ...COUNT_LABELS.map(([k, label]) => ({ label, backup: String(m.counts[k]), now: String(r.current_counts[k]) })),
    { label: "Library files", backup: m.library_included ? "included" : "not included (kept as is)", now: "" },
  ];
  return rows.map((x) => ({ ...x, differs: x.now !== "" && x.now !== "now" && x.backup !== x.now }));
}

/** One-line summary of a backup's contents. */
export function countsLine(c: BackupCounts): string {
  return `${c.printers} printers · ${c.spools} spools · ${c.filaments} filaments · ${c.jobs} jobs`;
}
