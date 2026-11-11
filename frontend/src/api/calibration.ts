/** Calibration (task 5.4, W10): skew per printer, shrinkage per filament. */
import { request } from "./client";
import type { MacroPatch } from "./types/MacroPatch";
import type { Plane } from "./types/Plane";
import type { PlaneMeasure } from "./types/PlaneMeasure";
import type { PlaneSkew } from "./types/PlaneSkew";
import type { ShrinkageInput } from "./types/ShrinkageInput";
import type { ShrinkageRecord } from "./types/ShrinkageRecord";
import type { ShrinkageResult } from "./types/ShrinkageResult";
import type { ShrinkFilament } from "./types/ShrinkFilament";
import type { ShrinkSaved } from "./types/ShrinkSaved";
import type { SkewApplied } from "./types/SkewApplied";
import type { SkewPreview } from "./types/SkewPreview";
import type { SkewPrinter } from "./types/SkewPrinter";
import type { SkewRecord } from "./types/SkewRecord";

export type {
  MacroPatch, Plane, PlaneMeasure, PlaneSkew, ShrinkageInput, ShrinkageRecord, ShrinkageResult, ShrinkFilament,
  ShrinkSaved, SkewApplied, SkewPreview, SkewPrinter, SkewRecord,
};

export const calibrationApi = {
  skewPrinters: () => request<SkewPrinter[]>("GET", "/api/calibration/skew"),
  skewPreview: (planes: PlaneMeasure[]) => request<SkewPreview>("POST", "/api/calibration/skew/preview", { planes }),
  saveMeasurement: (printerId: number, planes: PlaneMeasure[]) =>
    request<SkewRecord>("POST", `/api/calibration/skew/${printerId}/measurements`, { planes }),
  /** Sends SET_SKEW, SKEW_PROFILE SAVE=foreman, SAVE_CONFIG — only from the owner's Apply button. */
  apply: (printerId: number, recordId: number) =>
    request<SkewApplied>("POST", `/api/calibration/skew/${printerId}/apply`, { record_id: recordId, confirm_restart: true }),
  /** Sends SET_SKEW CLEAR=1 — only from the owner's button. */
  clear: (printerId: number) => request<{ sent: string }>("POST", `/api/calibration/skew/${printerId}/clear`, {}),
  macrosPreview: (printerId: number) => request<MacroPatch>("GET", `/api/calibration/skew/${printerId}/macros`),
  macrosApply: (printerId: number, patch: MacroPatch, restart: boolean) =>
    request<{ written: string[]; restarted: boolean }>("POST", `/api/calibration/skew/${printerId}/macros`, {
      approved: patch.files.map((f) => ({ path: f.path, sha: f.sha })),
      restart,
    }),
  shrinkFilaments: () => request<ShrinkFilament[]>("GET", "/api/calibration/shrinkage"),
  shrinkPreview: (input: ShrinkageInput) => request<ShrinkageResult>("POST", "/api/calibration/shrinkage/preview", input),
  saveShrinkage: (filamentId: number, input: ShrinkageInput, printerId: number | null) =>
    request<ShrinkSaved>("POST", `/api/calibration/shrinkage/${filamentId}`, { input, printer_id: printerId }),
};
