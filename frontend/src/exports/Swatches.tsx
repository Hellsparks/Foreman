/**
 * Tools → Swatches (task 5.5, D8). Pick a filament (or type), check the fit,
 * preview, download one STL — or tick several filaments for a zip.
 * The STL is built on the server (crates/foreman-swatch); the preview is to scale.
 */
import { useEffect, useMemo, useState } from "react";
import { exportsApi, type ExportRow, type SwatchFit } from "../api/exports";
import { BANDS, label, luminance, matches, pctX, pctY, plural, TEXT_X } from "./exportLogic";

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export default function Swatches({ rows, onError }: { rows: ExportRow[]; onError: (e: string | null) => void }) {
  const [pick, setPick] = useState<number | null>(null);
  const [l1, setL1] = useState("");
  const [l2, setL2] = useState("");
  const [fit, setFit] = useState<SwatchFit | null>(null);
  const [busy, setBusy] = useState(false);
  const [batch, setBatch] = useState<Set<number>>(new Set());
  const [q, setQ] = useState("");

  const row = rows.find((r) => r.id === pick) ?? null;
  useEffect(() => {
    if (pick === null && rows.length > 0) choose(rows[0]);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rows]);

  function choose(r: ExportRow) {
    setPick(r.id);
    setL1(r.swatch_line1);
    setL2(r.swatch_line2);
  }

  useEffect(() => {
    const t = setTimeout(() => {
      exportsApi.swatchFit(l1, l2).then(setFit, (e) => onError(errText(e)));
    }, 150);
    return () => clearTimeout(t);
  }, [l1, l2, onError]);

  const hex = row?.colours[0] ?? null;
  const dark = hex ? luminance(hex) < 0.5 : false;
  const shrunk = [fit?.line1, fit?.line2].filter((f) => f?.shrunk);
  const fitNote = fit?.error
    ? { tone: "warn", text: `⚠ ${fit.error}` }
    : shrunk.length > 0
      ? {
          tone: "warn",
          text: `⚠ Long text: ${shrunk.map((f) => `“${f?.text}” shrinks to ${f?.font_size.toFixed(1)} mm`).join(", ")} to fit ${fit?.max_width} mm.`,
        }
      : { tone: "ok", text: `✓ Fits at full size (${fit?.full_size ?? 6.4} mm text).` };

  async function downloadOne() {
    setBusy(true);
    try {
      await exportsApi.swatchStl(l1, l2);
      onError(null);
    } catch (e) {
      onError(errText(e));
    } finally {
      setBusy(false);
    }
  }

  async function downloadBatch() {
    setBusy(true);
    try {
      await exportsApi.swatchBatch([...batch]);
      onError(null);
    } catch (e) {
      onError(errText(e));
    } finally {
      setBusy(false);
    }
  }

  const shown = useMemo(() => rows.filter((r) => matches(r, q)), [rows, q]);
  const toggle = (id: number) =>
    setBatch((s) => {
      const n = new Set(s);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });

  const line = (f: SwatchFit["line1"], y: number, text: string) =>
    text.trim() && (
      <div
        className="sw-line"
        style={{
          left: `${pctX(TEXT_X)}%`,
          top: `${100 - pctY(y)}%`,
          fontSize: `calc(${(f?.font_size ?? 6.4) / 75} * 100cqw)`,
        }}
      >
        {text.trim()}
      </div>
    );

  return (
    <div className="tl-cal">
      <div className="sw-wrap">
        <section className="tl-sec sw-form">
          <h2 className="tl-h2">Swatch text</h2>
          <label className="tl-field">
            <span className="tl-label">Filament</span>
            <select
              className="tl-input"
              value={pick ?? ""}
              onChange={(e) => {
                const r = rows.find((x) => x.id === Number(e.target.value));
                if (r) choose(r);
              }}
            >
              {rows.map((r) => (
                <option key={r.id} value={r.id}>
                  {label(r)} · #{r.id}
                </option>
              ))}
            </select>
          </label>
          <label className="tl-field">
            <span className="tl-label">Line 1 · brand + material</span>
            <input className="tl-input" value={l1} onChange={(e) => setL1(e.target.value)} />
          </label>
          <label className="tl-field">
            <span className="tl-label">Line 2 · colour</span>
            <input className="tl-input" value={l2} onChange={(e) => setL2(e.target.value)} />
          </label>
          <span className={`tl-small tl-fg--${fitNote.tone}`} role="status">
            {fitNote.text}
          </span>
          <div className="tl-row">
            <button type="button" className="tl-btn tl-btn--primary sw-grow" disabled={busy || !!fit?.error} onClick={() => void downloadOne()}>
              Download STL
            </button>
          </div>
        </section>
        <section aria-label="Preview" className="tl-sec sw-preview">
          <div
            role="img"
            aria-label={`Swatch preview: ${l1} / ${l2}`}
            className={`sw-plate ${dark ? "sw-plate--dark" : "sw-plate--light"}`}
            style={hex ? ({ "--sw-colour": `#${hex}` } as React.CSSProperties) : undefined}
          >
            <span className="sw-hole" style={{ left: `${pctX(5)}%`, top: `${100 - pctY(35)}%` }} />
            {line(fit?.line1 ?? null, BANDS.upper, l1)}
            <div className="sw-groove" style={{ top: `${100 - pctY(29)}%` }} />
            {line(fit?.line2 ?? null, BANDS.lower, l2)}
          </div>
          <span className="tl-mono tl-small tl-fg--muted">75 × 40 × 3 mm · text debossed 0.6 mm · Liberation Sans</span>
        </section>
      </div>

      <section className="tl-sec">
        <h2 className="tl-h2">Several at once</h2>
        <p className="tl-muted tl-small">Tick filaments to get one STL each, with their default text, in a zip.</p>
        <div className="tl-row">
          <input className="tl-input sw-search" placeholder="Search filaments" value={q} onChange={(e) => setQ(e.target.value)} aria-label="Search filaments" />
          <button type="button" className="tl-btn tl-btn--primary" disabled={busy || batch.size === 0} onClick={() => void downloadBatch()}>
            {batch.size === 0 ? "Download swatches" : `Download ${plural(batch.size, "swatch", "swatches")} (zip)`}
          </button>
          {batch.size > 0 && (
            <button type="button" className="tl-btn tl-btn--small" onClick={() => setBatch(new Set())}>
              Clear
            </button>
          )}
        </div>
        <ul className="tl-list ex-list">
          {shown.map((r) => (
            <li key={r.id} className="tl-list__row">
              <label className="tl-check ex-check">
                <input type="checkbox" checked={batch.has(r.id)} onChange={() => toggle(r.id)} />
                <span className="ex-dot" style={r.colours[0] ? ({ "--sw-colour": `#${r.colours[0]}` } as React.CSSProperties) : undefined} aria-hidden="true" />
                <span>{label(r)}</span>
              </label>
              <span className="tl-mono tl-small tl-fg--muted">
                {r.swatch_line1} / {r.swatch_line2}
              </span>
            </li>
          ))}
        </ul>
      </section>
    </div>
  );
}
