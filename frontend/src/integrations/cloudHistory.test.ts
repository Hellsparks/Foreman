import { describe, expect, it } from "vitest";
import type { CloudTask } from "../api/bambu";
import { taskChip, taskSeconds } from "./CloudHistory";

const task = (p: Partial<CloudTask>): CloudTask => ({
  id: 1, title: "_KX139_BodyCentre", device_id: "03919C461200718", device_name: "Hellspark A1",
  status: 3, outcome: "failed", start_time: "2026-11-01T19:24:11Z", end_time: "2026-11-01T19:52:49Z",
  weight_g: 49.74, plate_name: null, cover: null, ...p,
});

describe("cloud history rows", () => {
  it("uses wall-clock time, not Bambu's slice estimate", () => {
    expect(taskSeconds(task({}))).toBe(1718);
    expect(taskSeconds(task({ end_time: null }))).toBeNull();
  });
  it("labels outcomes", () => {
    expect(taskChip(task({})).word).toBe("Failed / cancelled");
    expect(taskChip(task({ outcome: "finished", status: 2 })).tone).toBe("ok");
    expect(taskChip(task({ outcome: "unknown", status: 7 })).word).toBe("Status 7");
  });
});
