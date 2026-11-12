import { describe, expect, it } from "vitest";
import type { FoundPrinter, PendingImport } from "../api/setup";
import {
  addLabel,
  checklistHeading,
  countsLine,
  foundSubline,
  missingFor,
  NO_CREDS,
  readinessChip,
  revisionLine,
  scannedAgo,
  stepIndex,
  validAccessCode,
  validSerial,
} from "./setupLogic";

const moon: FoundPrinter = {
  key: "moonraker:10.0.0.177",
  firmware: "moonraker",
  name: "HellV0",
  host: "10.0.0.177",
  url: "http://10.0.0.177",
  serial: null,
  model: "Klipper",
  version: "v0.11.0",
  needs: "ready",
  via: ["mdns"],
  checks: [],
  added_as: null,
};
const bambu: FoundPrinter = {
  ...moon,
  key: "bambu:10.0.0.249",
  firmware: "bambu",
  name: "Bambu A1",
  host: "10.0.0.249",
  url: "10.0.0.249",
  serial: "03919C461200718",
  model: "A1",
  version: null,
  needs: "access_code",
};

describe("setup logic", () => {
  it("orders steps", () => {
    expect(stepIndex("data")).toBe(0);
    expect(stepIndex("finish")).toBe(2);
    expect(stepIndex("done")).toBe(3);
  });

  it("formats the checklist heading like FleetEmpty", () => {
    expect(checklistHeading(2, 4)).toBe("Getting started · 2 of 4 done");
    expect(checklistHeading(5, 5)).toBe("Getting started · all done");
  });

  it("summarises counts", () => {
    expect(countsLine({ spools: 90, filaments: 288, vendors: 64, archived_spools: 0, extra_fields: 5 })).toBe(
      "90 spools · 288 filaments · 64 vendors",
    );
    expect(countsLine({ spools: 1, filaments: 1, vendors: 1, archived_spools: 1, extra_fields: 0 })).toBe(
      "1 spool · 1 filament · 1 vendor · 1 archived",
    );
  });

  it("explains revisions", () => {
    const p = { revision: "415a8f855e14", status: "older", steps_to_head: 2, head: "fe4970567bb3" } as PendingImport;
    expect(revisionLine(p)).toContain("2 migration steps");
    expect(revisionLine({ ...p, status: "unknown", revision: "ffff" })).toBe("Revision ffff · unknown (newer than fe4970567bb3)");
  });

  it("labels found printers", () => {
    expect(foundSubline(moon)).toBe("10.0.0.177 · Moonraker v0.11.0 · Klipper");
    expect(foundSubline(bambu)).toBe("10.0.0.249 · Bambu A1 · serial 03919C461200718");
    expect(readinessChip(moon).word).toBe("Ready");
    expect(readinessChip(bambu).word).toBe("Access code");
    expect(readinessChip({ ...moon, added_as: 3 }).word).toBe("Added");
  });

  it("asks for the right credentials before adding", () => {
    expect(missingFor(moon, NO_CREDS)).toBeNull();
    expect(missingFor({ ...moon, needs: "api_key" }, NO_CREDS)).toMatch(/API key/);
    expect(missingFor({ ...moon, needs: "api_key" }, { ...NO_CREDS, apiKey: "k" })).toBeNull();
    expect(missingFor(bambu, NO_CREDS)).toMatch(/access code/);
    expect(missingFor(bambu, { ...NO_CREDS, accessCode: "12ab34cd" })).toBeNull();
    expect(missingFor({ ...bambu, serial: null }, { ...NO_CREDS, accessCode: "12ab34cd" })).toMatch(/serial/);
    expect(missingFor({ ...moon, added_as: 1 }, NO_CREDS)).toBe("Already added");
  });

  it("validates codes and serials", () => {
    expect(validAccessCode("12345678")).toBe(true);
    expect(validAccessCode("1234567")).toBe(false);
    expect(validSerial("03919C461200718")).toBe(true);
    expect(validSerial("03919c461200718")).toBe(false);
  });

  it("formats scan age and the add button", () => {
    const t = Date.parse("2026-11-12T10:00:00Z");
    expect(scannedAgo("2026-11-12T09:59:48Z", t)).toBe("Scanned 12 s ago");
    expect(scannedAgo("2026-11-12T09:55:00Z", t)).toBe("Scanned 5 min ago");
    expect(addLabel(2)).toBe("Add 2 printers");
    expect(addLabel(1)).toBe("Add 1 printer");
  });
});
