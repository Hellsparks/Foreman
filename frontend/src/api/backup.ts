/** Backup + restore (task 6.2, W14). */
import { request } from "./client";
import type { BackupCounts } from "./types/BackupCounts";
import type { BackupEntry } from "./types/BackupEntry";
import type { BackupKind } from "./types/BackupKind";
import type { BackupOverview } from "./types/BackupOverview";
import type { BackupSettingsDto } from "./types/BackupSettingsDto";
import type { CheckDirResult } from "./types/CheckDirResult";
import type { Current } from "./types/Current";
import type { InspectResult } from "./types/InspectResult";
import type { LastRun } from "./types/LastRun";
import type { Manifest } from "./types/Manifest";
import type { RestoreStarted } from "./types/RestoreStarted";
import type { Schedule } from "./types/Schedule";

export type {
  BackupCounts,
  BackupEntry,
  BackupKind,
  BackupOverview,
  BackupSettingsDto,
  CheckDirResult,
  Current,
  InspectResult,
  LastRun,
  Manifest,
  RestoreStarted,
  Schedule,
};

export const backupApi = {
  overview: () => request<BackupOverview>("GET", "/api/backup"),
  /** `passphrase`: undefined keeps the saved one, "" removes it. */
  save: (settings: BackupSettingsDto, passphrase?: string) =>
    request<BackupOverview>("PUT", "/api/backup/settings", passphrase === undefined ? { settings } : { settings, passphrase }),
  checkDir: (dir: string) => request<CheckDirResult>("POST", "/api/backup/check-dir", { dir }),
  run: (passphrase?: string) => request<LastRun>("POST", "/api/backup/run", passphrase ? { passphrase } : {}),
  inspect: (path: string, passphrase?: string) =>
    request<InspectResult>("POST", "/api/backup/inspect", passphrase ? { path, passphrase } : { path }),
  restore: (path: string, passphrase?: string) =>
    request<RestoreStarted>("POST", "/api/backup/restore", passphrase ? { path, passphrase } : { path }),
  /** Health without the 401 → sign-in redirect (used while the server restarts). */
  alive: async () => {
    try {
      await request<unknown>("GET", "/api/health", undefined, undefined, false);
      return true;
    } catch {
      return false;
    }
  },
};
