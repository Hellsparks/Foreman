/** Exports + swatches (task 5.5, W11). */
import { postBlob, request, saveBlob } from "./client";
import type { ExportRow } from "./types/ExportRow";
import type { HueForgeSkip } from "./types/HueForgeSkip";
import type { OrcaProfileInfo } from "./types/OrcaProfileInfo";
import type { SwatchFit } from "./types/SwatchFit";
import type { SwatchLineFit } from "./types/SwatchLineFit";

export type { ExportRow, HueForgeSkip, OrcaProfileInfo, SwatchFit, SwatchLineFit };

async function download(path: string, body: unknown, fallback: string) {
  const { blob, filename } = await postBlob(path, body);
  saveBlob(blob, filename ?? fallback);
}

export const exportsApi = {
  rows: () => request<ExportRow[]>("GET", "/api/exports"),
  /** `null` = every filament. */
  orca: (ids: number[] | null) => download("/api/exports/orca", { filament_ids: ids }, "foreman-orca-filaments.zip"),
  hueforge: (ids: number[] | null) => download("/api/exports/hueforge", { filament_ids: ids }, "foreman-hueforge.json"),
  swatchFit: (line1: string, line2: string) => request<SwatchFit>("POST", "/api/swatch/fit", { line1, line2 }),
  swatchStl: (line1: string, line2: string) => download("/api/swatch/stl", { line1, line2 }, "swatch.stl"),
  swatchBatch: (ids: number[]) => download("/api/swatch/batch", { filament_ids: ids }, "foreman-swatches.zip"),
};
