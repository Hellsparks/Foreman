import { describe, expect, it } from "vitest";
import type { InspectResult } from "../api/types/InspectResult";
import { compareRows, fmtBytes, scheduleText } from "./backupLogic";

const counts = { printers: 2, jobs: 10, projects: 1, library_files: 5, spools: 30, filaments: 89, vendors: 17 };

describe("backupLogic", () => {
  it("formats sizes", () => {
    expect(fmtBytes(512)).toBe("512 B");
    expect(fmtBytes(3_100_000)).toBe("3.0 MB");
    expect(fmtBytes(250 * 1024 * 1024)).toBe("250 MB");
  });
  it("describes the schedule", () => {
    expect(scheduleText({ schedule: "off", time: "03:00", weekday: 7 })).toBe("Off");
    expect(scheduleText({ schedule: "daily", time: "03:00", weekday: 7 })).toBe("Daily at 03:00");
    expect(scheduleText({ schedule: "weekly", time: "02:30", weekday: 7 })).toBe("Weekly on Sunday at 02:30");
    expect(scheduleText({ schedule: "weekly", time: "02:30", weekday: 1 })).toBe("Weekly on Monday at 02:30");
  });
  it("flags counts that differ from now", () => {
    const r: InspectResult = {
      manifest: {
        format: 1,
        kind: "manual",
        created_at: "2026-11-12T03:00:00Z",
        foreman_version: "0.1.0",
        foreman_schema: 90,
        spoolman_revision: "fe4970567bb3",
        host: "h",
        encrypted: false,
        library_included: false,
        counts,
        files: [],
        total_bytes: 1,
      },
      current: { foreman_version: "0.1.0", foreman_schema: 90, spoolman_revision: "fe4970567bb3" },
      current_counts: { ...counts, spools: 31 },
      problem: null,
    };
    const rows = compareRows(r);
    expect(rows.find((x) => x.label === "Spools")?.differs).toBe(true);
    expect(rows.find((x) => x.label === "Filaments")?.differs).toBe(false);
    expect(rows.find((x) => x.label === "Foreman")?.differs).toBe(false);
  });
});
