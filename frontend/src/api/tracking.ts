/** Settings → Filament: tracking mode and the Spoolman API switch (doc 07 §A). */
import { request } from "./client";
import type { FilamentMode } from "./types/FilamentMode";
import type { FilamentPolicy } from "./types/FilamentPolicy";
import type { FilamentSettings } from "./types/FilamentSettings";
import type { SwitchPreview } from "./types/SwitchPreview";
import type { SwitchResult } from "./types/SwitchResult";

export type { FilamentMode, FilamentPolicy, FilamentSettings, SwitchPreview, SwitchResult };

export const trackingApi = {
  get: () => request<FilamentSettings>("GET", "/api/settings/filament"),
  preview: (mode: FilamentMode) =>
    request<SwitchPreview>("GET", `/api/settings/filament/preview?mode=${encodeURIComponent(mode)}`),
  setMode: (mode: FilamentMode) => request<SwitchResult>("PUT", "/api/settings/filament", { mode, confirm: true }),
  /** Setup wizard on a fresh install: Off with the API not served (nothing points at it yet). */
  setupOff: () => request<SwitchResult>("PUT", "/api/settings/filament", { mode: "off", serve_api: false, confirm: true }),
  setServeApi: (serve_api: boolean) => request<SwitchResult>("PUT", "/api/settings/filament", { serve_api }),
};
