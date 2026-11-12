/** First-run setup (task 6.4, W1). */
import { request } from "./client";
import type { AddPrinterRequest } from "./types/AddPrinterRequest";
import type { AddPrinterResult } from "./types/AddPrinterResult";
import type { Check } from "./types/Check";
import type { Checklist } from "./types/Checklist";
import type { ChecklistItem } from "./types/ChecklistItem";
import type { DataSource } from "./types/DataSource";
import type { DataSourceKind } from "./types/DataSourceKind";
import type { FoundPrinter } from "./types/FoundPrinter";
import type { ImportCounts } from "./types/ImportCounts";
import type { PendingImport } from "./types/PendingImport";
import type { ProbeRequest } from "./types/ProbeRequest";
import type { ProbeResult } from "./types/ProbeResult";
import type { Readiness } from "./types/Readiness";
import type { ScanResult } from "./types/ScanResult";
import type { SetupOverview } from "./types/SetupOverview";
import type { SetupStep } from "./types/SetupStep";
import type { SlicerKeyResult } from "./types/SlicerKeyResult";
import type { WizardState } from "./types/WizardState";

export type {
  AddPrinterRequest,
  AddPrinterResult,
  Check,
  Checklist,
  ChecklistItem,
  DataSource,
  DataSourceKind,
  FoundPrinter,
  ImportCounts,
  PendingImport,
  ProbeRequest,
  ProbeResult,
  Readiness,
  ScanResult,
  SetupOverview,
  SetupStep,
  SlicerKeyResult,
  WizardState,
};

export const setupApi = {
  get: () => request<SetupOverview>("GET", "/api/setup"),
  step: (step: SetupStep) => request<SetupOverview>("POST", "/api/setup/step", { step }),
  skip: () => request<SetupOverview>("POST", "/api/setup/skip"),
  restart: () => request<SetupOverview>("POST", "/api/setup/restart"),
  finish: () => request<SetupOverview>("POST", "/api/setup/finish"),
  fresh: () => request<SetupOverview>("POST", "/api/setup/data/fresh"),
  detectPath: (path: string) => request<PendingImport>("POST", "/api/setup/data/detect-path", { path }),
  detectUpload: (file: File) => {
    const f = new FormData();
    f.append("file", file, file.name);
    return request<PendingImport>("POST", "/api/setup/data/detect-upload", undefined, f);
  },
  detectUrl: (url: string) => request<PendingImport>("POST", "/api/setup/data/detect-url", { url }),
  importData: (replace_existing: boolean) =>
    request<SetupOverview>("POST", "/api/setup/data/import", { replace_existing }),
  discard: () => request<SetupOverview>("POST", "/api/setup/data/discard"),
  discover: () => request<ScanResult>("POST", "/api/setup/discover"),
  probe: (req: ProbeRequest) => request<ProbeResult>("POST", "/api/setup/probe", req),
  addPrinter: (req: AddPrinterRequest) => request<AddPrinterResult>("POST", "/api/setup/printers", req),
  slicerKey: () => request<SlicerKeyResult>("POST", "/api/setup/slicer-key"),
  backup: (enable: boolean, dir?: string) =>
    request<SetupOverview>("POST", "/api/setup/backup", dir ? { enable, dir } : { enable }),
};
