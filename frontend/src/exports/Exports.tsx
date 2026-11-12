/**
 * Tools → Export (task 5.5, W11): OrcaSlicer filament profiles and the HueForge
 * catalogue, for every filament or a ticked selection.
 */
import { useMemo, useState } from "react";
import { exportsApi, type ExportRow } from "../api/exports";
import { exportSummary, label, matches, plural } from "./exportLogic";

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

const SKIP_TEXT = { missing_td: "no TD", multi_colour: "multi-colour", no_colour: "no colour" } as const;

export default function Exports({ rows, onError }: { rows: ExportRow[]; onError: (e: string | null) => void }) {
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [q, setQ] = useState("");
  const [busy, setBusy] = useState(false);
  const sum = exportSummary(rows, selected);
  const ids = selected.size > 0 ? [...selected] : null;
  const shown = useMemo(() => rows.filter((r) => matches(r, q)), [rows, q]);

  async function run(f: () => Promise<void>) {
    setBusy(true);
    try {
      await f();
      onError(null);
    } catch (e) {
      onError(errText(e));
    } finally {
      setBusy(false);
    }
  }

  const toggle = (id: number) =>
    setSelected((s) => {
      const n = new Set(s);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });
  const scopeText = selected.size > 0 ? "selected" : "all";

  return (
    <div className="tl-cal">
      <div className="ex-cards">
        <section className="tl-sec ex-card">
          <h2 className="tl-h2">OrcaSlicer filament profiles</h2>
          <p className="tl-muted tl-small ex-grow">
            One profile per filament with temps, shrinkage and flow from Foreman, on the matching Orca {"“"}Generic{"”"} or Bambu
            preset (that preset sets the Bambu filament id).
          </p>
          <span className="tl-small tl-fg--muted">
            {plural(sum.withShrink, "profile")} with shrinkage
            {sum.multiOrca > 0 && ` · ${plural(sum.multiOrca, "multi-colour filament")} export with all colours listed`}
          </span>
          <button type="button" className="tl-btn" disabled={busy || sum.scope === 0} onClick={() => void run(() => exportsApi.orca(ids))}>
            Export {plural(sum.scope, "profile")} ({scopeText})
          </button>
          <p className="tl-small tl-muted">
            In OrcaSlicer: <b>File → Import → Import Configs…</b> and pick the zip. The profiles appear under the filament list as user
            presets. Re-importing asks before overwriting.
          </p>
        </section>
        <section className="tl-sec ex-card">
          <h2 className="tl-h2">HueForge catalogue</h2>
          <p className="tl-muted tl-small ex-grow">Filaments with a transmission distance (TD) for HueForge.</p>
          <span className={`tl-small ${sum.hueMulti > 0 ? "tl-fg--warn" : "tl-fg--muted"}`}>
            {sum.hueMulti > 0 ? `⚠ ${plural(sum.hueMulti, "multi-colour filament")} skipped (need one TD)` : "One TD per filament."}
          </span>
          <button type="button" className="tl-btn" disabled={busy || sum.hue === 0} onClick={() => void run(() => exportsApi.hueforge(ids))}>
            Export {plural(sum.hue, "filament")} ({scopeText})
          </button>
          <p className="tl-small tl-muted">
            Import the JSON into HueForge's filament library. Each filament keeps the same id on every export.
          </p>
        </section>
        <section className="tl-sec ex-card">
          <h2 className="tl-h2">CSV</h2>
          <p className="tl-muted tl-small ex-grow">Every spool with vendor, filament, colour, weights, location and prices, for spreadsheets.</p>
          <span className="tl-small tl-fg--muted">All spools; archived ones are marked.</span>
          <button type="button" className="tl-btn" disabled={busy} onClick={() => void run(() => exportsApi.spoolsCsv())}>
            Download CSV
          </button>
        </section>
        <section className="tl-sec ex-card">
          <h2 className="tl-h2">Spoolman JSON</h2>
          <p className="tl-muted tl-small ex-grow">
            Vendors, filaments and spools exactly as Spoolman{"’"}s API returns them, extra fields included.
          </p>
          <span className="tl-small tl-fg--muted">One file; archived spools included.</span>
          <button type="button" className="tl-btn" disabled={busy} onClick={() => void run(() => exportsApi.spoolmanJson())}>
            Download JSON
          </button>
          <p className="tl-small tl-muted">
            Spoolman itself has no import; scripts can post these records to any Spoolman{"’"}s <span className="tl-mono">/api/v1</span>.
          </p>
        </section>
      </div>

      <section className="tl-sec">
        <h2 className="tl-h2">Filaments</h2>
        <div className="tl-row">
          <input className="tl-input sw-search" placeholder="Search filaments" value={q} onChange={(e) => setQ(e.target.value)} aria-label="Search filaments" />
          <button type="button" className="tl-btn tl-btn--small" onClick={() => setSelected(new Set(shown.map((r) => r.id)))}>
            Select shown
          </button>
          {selected.size > 0 && (
            <button type="button" className="tl-btn tl-btn--small" onClick={() => setSelected(new Set())}>
              Clear ({selected.size})
            </button>
          )}
        </div>
        <p className="tl-muted tl-small">Nothing ticked = export everything.</p>
        <ul className="tl-list ex-list">
          {shown.map((r) => (
            <li key={r.id} className="tl-list__row">
              <label className="tl-check ex-check">
                <input type="checkbox" checked={selected.has(r.id)} onChange={() => toggle(r.id)} />
                <span className="ex-dot" style={r.colours[0] ? ({ "--sw-colour": `#${r.colours[0]}` } as React.CSSProperties) : undefined} aria-hidden="true" />
                <span>{label(r)}</span>
              </label>
              <span className="tl-mono tl-small tl-fg--muted">
                {r.orca.inherits.replace(" @System", "")}
                {r.orca.bambu_filament_id && ` · ${r.orca.bambu_filament_id}`}
                {r.orca.shrinkage_xy != null && ` · XY ${r.orca.shrinkage_xy}%`}
                {r.orca.shrinkage_z != null && ` · Z ${r.orca.shrinkage_z}%`}
              </span>
              <span className={`tl-mono tl-small ${r.hueforge_skip ? "tl-fg--muted" : "tl-fg--ok"}`}>
                {r.hueforge_skip ? `HueForge: ${SKIP_TEXT[r.hueforge_skip]}` : `TD ${r.hue_td}`}
              </span>
              {r.orca.notes.map((n) => (
                <span key={n} className="tl-small tl-fg--warn ex-note">
                  {n}
                </span>
              ))}
            </li>
          ))}
        </ul>
      </section>

      <section className="tl-sec">
        <h2 className="tl-h2">Measure missing TD</h2>
        <p className="tl-muted tl-small">
          {plural(sum.missingTd.length, "single-colour filament")} without a TD value. Owned ones first; measure them with a TD meter or HueForge's
          own test and enter the value on the filament.
        </p>
        <ul className="tl-list ex-list">
          {[...sum.missingTd]
            .sort((a, b) => Number(b.owned) - Number(a.owned))
            .map((r) => (
              <li key={r.id} className="tl-list__row">
                <span className="ex-dot" style={r.colours[0] ? ({ "--sw-colour": `#${r.colours[0]}` } as React.CSSProperties) : undefined} aria-hidden="true" />
                <span>{label(r)}</span>
                <span className="tl-mono tl-small tl-fg--muted">
                  #{r.id}
                  {r.owned ? " · owned" : ""}
                </span>
              </li>
            ))}
        </ul>
      </section>
    </div>
  );
}
