import { describe, expect, it } from "vitest";
import { amount, everyText, gapText, statusText, type ServiceLog, type TaskView } from "./maintenance";

const view = (over: Partial<TaskView["progress"]>, snooze = 0): TaskView => ({
  task: { id: 1, printer_id: 1, name: "Belt tension", notes: null, basis: "hours", interval: 200, every_days: null, snooze, since: null, created_at: "", updated_at: "" },
  printer_name: "HellV0",
  counters: { hours: 212, grams: 0, jobs: 0 },
  progress: { value: 212, limit: 200, days: 3, days_limit: null, ratio: 1.06, by_days: false, state: "overdue", ...over },
  last_done: null,
  snooze_amount: 10,
});

describe("maintenance formatting", () => {
  it("status text per state", () => {
    expect(statusText(view({}))).toBe("✖ Overdue · 212 / 200 h");
    expect(statusText(view({ value: 185, ratio: 0.925, state: "soon" }))).toBe("⚠ Due soon · 185 / 200 h");
    expect(statusText(view({ value: 0.02, ratio: 0, state: "ok" }))).toBe("✓ 0.02 / 200 h");
    expect(statusText(view({ days: 31, days_limit: 30, by_days: true }))).toBe("✖ Overdue · 31 / 30 days");
    expect(statusText(view({ value: 205, limit: 210, ratio: 0.97, state: "soon" }, 0.05))).toBe("⚠ Due soon · 205 / 210 h · snoozed");
  });

  it("amounts and intervals", () => {
    expect(amount(1500, "grams")).toBe("1.50 kg");
    expect(amount(0.18, "grams")).toBe("0.18 g");
    expect(amount(1, "jobs")).toBe("1 job");
    expect(everyText({ basis: "grams", interval: 5000, every_days: null })).toBe("5 kg filament");
    expect(everyText({ basis: "hours", interval: 300, every_days: 180 })).toBe("300 print hours · or 180 days");
  });

  it("gap between services", () => {
    const l: ServiceLog = { id: 1, task_id: 1, printer_id: 1, task_name: "Belt tension", basis: "hours", interval: 200, counter: 212, days: 41.4, first: false, done_at: "", done_by: "admin", notes: null };
    expect(gapText(l)).toBe("212 h · 41 days after previous");
    expect(gapText({ ...l, first: true, days: 0.2, basis: "jobs", counter: 2 })).toBe("2 jobs · < 1 day since counting started");
  });
});
