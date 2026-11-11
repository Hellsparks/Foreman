/** Tools (W10, W11). Reference: docs/ui-ref/Tools.dc.html. Calibration (5.4), Swatches + Export (5.5); Import comes later. */
import { useCallback, useEffect, useState } from "react";
import { exportsApi, type ExportRow } from "../api/exports";
import Calibration from "../calibration/Calibration";
import Exports from "../exports/Exports";
import Swatches from "../exports/Swatches";
import "./Tools.css";

type Tab = "cal" | "swatch" | "import" | "export";
const TABS: [Tab, string][] = [
  ["cal", "Calibration"],
  ["swatch", "Swatches"],
  ["import", "Import"],
  ["export", "Export"],
];

export default function Tools() {
  const [tab, setTab] = useState<Tab>("cal");
  const [rows, setRows] = useState<ExportRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const needRows = tab === "swatch" || tab === "export";
  useEffect(() => {
    if (!needRows) return;
    exportsApi.rows().then(
      (r) => {
        setRows(r);
        setError(null);
      },
      (e) => setError(e instanceof Error ? e.message : String(e)),
    );
  }, [needRows]);
  const onError = useCallback((e: string | null) => setError(e), []);

  return (
    <div className="tl">
      <header className="tl-head">
        <h1 className="tl-title phone-dup-title">Tools</h1>
        <div role="tablist" aria-label="Tools" className="tl-tabs">
          {TABS.map(([id, label]) => (
            <button key={id} type="button" role="tab" aria-selected={tab === id} className="tl-tab" onClick={() => setTab(id)}>
              {label}
            </button>
          ))}
        </div>
      </header>
      {needRows && error && (
        <p className="tl-error" role="alert">
          {error}
        </p>
      )}
      {tab === "cal" ? (
        <Calibration />
      ) : tab === "import" ? (
        <p className="tl-muted">Coming in a later session.</p>
      ) : rows === null ? (
        <p className="tl-muted">Loading filaments…</p>
      ) : tab === "swatch" ? (
        <Swatches rows={rows} onError={onError} />
      ) : (
        <Exports rows={rows} onError={onError} />
      )}
    </div>
  );
}
