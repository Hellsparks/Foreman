/**
 * Filament page — Spools tab + spool inspector (task 4.1).
 * Reference: docs/ui-ref/Filament.dc.html lines 65–121 (list) and 222–260 (inspector).
 * Filaments / Inventory / Vendors / Locations come next session.
 */
import { useCallback, useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { useOutletContext } from "react-router-dom";
import type { ShellContext } from "../shell/Shell";
import { useEventStream } from "../hooks/useEventStream";
import {
  colourKind,
  grams,
  leftFraction,
  LOW_GRAMS,
  materialLabel,
  SPOOL_TOPICS,
  spools,
  whereLabel,
  type SpoolRow,
} from "../api/spools";
import SpoolSwatch from "../filament/SpoolSwatch";
import LoadSpoolPanel from "../filament/LoadSpoolPanel";
import "../filament/filament.css";

const TABS = [
  ["spools", "Spools"],
  ["filaments", "Filaments"],
  ["inventory", "Inventory"],
  ["vendors", "Vendors"],
  ["locations", "Locations"],
] as const;
type Tab = (typeof TABS)[number][0];

type Mode = "spool" | "load" | "edit";

export default function Filament() {
  const { openInspector, closeInspector, inspectorOpen, inspectorBodyId } =
    useOutletContext<ShellContext>();
  const [tab, setTab] = useState<Tab>("spools");
  const [rows, setRows] = useState<SpoolRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [q, setQ] = useState("");
  const [mat, setMat] = useState("all");
  const [low, setLow] = useState(false);
  const [showArchived, setShowArchived] = useState(false);
  const [sel, setSel] = useState<number | null>(null);
  const [mode, setMode] = useState<Mode>("spool");

  const load = useCallback(async () => {
    try {
      setRows(await spools.list(showArchived));
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [showArchived]);
  useEffect(() => void load(), [load]);
  const onSpoolEvent = useCallback(() => void load(), [load]);
  useEventStream(onSpoolEvent, SPOOL_TOPICS);
  useEffect(() => () => closeInspector(), [closeInspector]);

  const [portal, setPortal] = useState<Element | null>(null);
  useEffect(() => setPortal(document.getElementById(inspectorBodyId)), [inspectorBodyId, inspectorOpen]);

  const all = rows ?? [];
  const materials = useMemo(() => {
    const count = new Map<string, number>();
    for (const r of all) {
      const m = (r.spool.material ?? "").toUpperCase();
      if (m) count.set(m, (count.get(m) ?? 0) + 1);
    }
    return [...count.entries()].sort((a, b) => b[1] - a[1]).slice(0, 5).map(([m]) => m);
  }, [all]);

  const shown = all.filter((r) => {
    const s = r.spool;
    if (mat !== "all" && (s.material ?? "").toUpperCase() !== mat) return false;
    if (low && !(s.remaining_weight != null && s.remaining_weight < LOW_GRAMS)) return false;
    if (!q.trim()) return true;
    const hay = [`#${s.id}`, s.name, s.vendor, s.material, s.modifier, s.location, s.color_hex, s.lot_nr, ...r.loaded.map((l) => l.printer_name)]
      .filter(Boolean)
      .join(" ")
      .toLowerCase();
    return q.toLowerCase().replace(/#(?=[0-9a-f]{6})/g, "").split(/\s+/).filter(Boolean).every((w) => hay.includes(w));
  });

  const kg = all.reduce((a, r) => a + (r.spool.remaining_weight ?? 0), 0) / 1000;
  const lowCount = all.filter((r) => r.spool.remaining_weight != null && r.spool.remaining_weight < LOW_GRAMS).length;
  const cur = all.find((r) => r.spool.id === sel) ?? null;

  const open = (id: number, m: Mode = "spool") => {
    setSel(id);
    setMode(m);
    openInspector();
  };
  const close = () => {
    setSel(null);
    closeInspector();
  };

  return (
    <div className="fil-page">
      <header className="fil-head">
        <div>
          <h1 className="fil-head__title">Filament</h1>
          <p className="fil-head__sub">
            {rows == null
              ? "Loading spools…"
              : `${all.length} spool${all.length === 1 ? "" : "s"} · ${kg.toFixed(1)} kg on hand · ${lowCount} low`}
          </p>
        </div>
        <div role="tablist" aria-label="Filament sections" className="fil-tabs">
          {TABS.map(([id, label]) => (
            <button key={id} type="button" role="tab" aria-selected={tab === id} className="fil-tab" onClick={() => setTab(id)}>
              {label}
            </button>
          ))}
        </div>
      </header>

      <main className="fil-main">
        {error && <p className="fil-note fil-note--bad" role="alert">{error}</p>}
        {tab !== "spools" ? (
          <p className="fil-placeholder">
            {TABS.find((t) => t[0] === tab)?.[1]} comes next session (filament form, inventory, vendors, locations).
          </p>
        ) : (
          <>
            <div className="fil-filters">
              <input
                type="search"
                className="fil-input"
                aria-label="Search spools"
                placeholder="#id, name, colour, location…"
                value={q}
                onChange={(e) => setQ(e.target.value)}
              />
              <div role="group" aria-label="Material" className="fil-segs">
                {["all", ...materials].map((m) => (
                  <button key={m} type="button" className="fil-seg" aria-pressed={mat === m} onClick={() => setMat(m)}>
                    {m === "all" ? "All" : m}
                  </button>
                ))}
              </div>
              <button type="button" className="fil-seg" aria-pressed={low} onClick={() => setLow(!low)}>
                Low · under {LOW_GRAMS} g
              </button>
              <button type="button" className="fil-seg" aria-pressed={showArchived} onClick={() => setShowArchived(!showArchived)}>
                Show archived
              </button>
            </div>
            <div className="fil-count">{shown.length} shown</div>
            <div className="fil-table-wrap">
              <table className="fil-table">
                <thead>
                  <tr>
                    <th>Spool</th>
                    <th>Material</th>
                    <th style={{ width: 220 }}>Left</th>
                    <th>Where</th>
                  </tr>
                </thead>
                <tbody>
                  {shown.map((r) => {
                    const s = r.spool;
                    const isLow = s.remaining_weight != null && s.remaining_weight < LOW_GRAMS;
                    return (
                      <tr key={s.id} aria-selected={sel === s.id} className="fil-tr" onClick={() => open(s.id)}>
                        <td>
                          <button
                            type="button"
                            className="fil-row-btn"
                            onClick={(e) => {
                              e.stopPropagation();
                              open(s.id);
                            }}
                          >
                            <SpoolSwatch spool={s} size={28} />
                            <span>
                              <b>
                                #{s.id} {s.name}
                              </b>
                              <br />
                              <span className="fil-sub">
                                {s.vendor ?? "No vendor"}
                                {s.archived ? " · archived" : ""}
                              </span>
                            </span>
                          </button>
                        </td>
                        <td>
                          <span className="fil-mono">{materialLabel(s)}</span>
                          {s.multi_color_hexes.length > 1 && (
                            <>
                              <br />
                              <span className="fil-sub">{colourKind(s)}</span>
                            </>
                          )}
                          {s.abrasive && (
                            <>
                              <br />
                              <span className="fil-abrasive">abrasive</span>
                            </>
                          )}
                        </td>
                        <td>
                          <div className="fil-bar" aria-hidden="true">
                            <div
                              className={`fil-bar__fill${isLow ? " fil-bar__fill--low" : ""}`}
                              style={{ width: `${Math.round(leftFraction(s) * 100)}%` }}
                            />
                          </div>
                          <div className={`fil-left${isLow ? " fil-left--low" : ""}`}>
                            {grams(s.remaining_weight)}
                            {isLow ? " · low" : ""}
                          </div>
                        </td>
                        <td>{whereLabel(s, r.loaded)}</td>
                      </tr>
                    );
                  })}
                  {rows != null && shown.length === 0 && (
                    <tr>
                      <td colSpan={4} style={{ padding: 16 }} className="fil-placeholder">
                        {all.length ? "No spool matches." : "No spools in Spoolman yet."}
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
          </>
        )}
      </main>

      {portal &&
        inspectorOpen &&
        cur &&
        createPortal(
          mode === "load" ? (
            <LoadSpoolPanel spoolId={cur.spool.id} onClose={() => setMode("spool")} onLoaded={() => void load()} />
          ) : mode === "edit" ? (
            <EditSpoolPanel row={cur} onClose={() => setMode("spool")} onSaved={() => void load().then(() => setMode("spool"))} />
          ) : (
            <SpoolInspector
              row={cur}
              onClose={close}
              onLoad={() => setMode("load")}
              onEdit={() => setMode("edit")}
              onArchive={async (archived) => {
                await spools.edit(cur.spool.id, { archived, location: null, lot_nr: null, comment: null, price: null });
                if (archived && !showArchived) close();
                await load();
              }}
            />
          ),
          portal,
        )}
    </div>
  );
}

// ── Spool inspector ───────────────────────────────────────────────────────────

function SpoolInspector({
  row,
  onClose,
  onLoad,
  onEdit,
  onArchive,
}: {
  row: SpoolRow;
  onClose: () => void;
  onLoad: () => void;
  onEdit: () => void;
  onArchive: (archived: boolean) => Promise<void>;
}) {
  const s = row.spool;
  const [confirmArchive, setConfirmArchive] = useState(false);
  const [archErr, setArchErr] = useState<string | null>(null);
  useEffect(() => setConfirmArchive(false), [s.id]);
  const archive = async () => {
    if (!s.archived && !confirmArchive) {
      setConfirmArchive(true);
      return;
    }
    setArchErr(null);
    try {
      await onArchive(!s.archived);
      setConfirmArchive(false);
    } catch (e) {
      setArchErr(e instanceof Error ? e.message : String(e));
    }
  };
  const isLow = s.remaining_weight != null && s.remaining_weight < LOW_GRAMS;
  const colours = s.multi_color_hexes.length > 1 ? s.multi_color_hexes.map((h) => `#${h}`).join(" ") : s.color_hex ? `#${s.color_hex}` : "no colour";
  const facts: [string, string][] = [
    ["Colour", `${colourKind(s)} · ${colours}`],
    ["Where", whereLabel(s, row.loaded)],
    ["Location", s.location || "—"],
    ["Diameter", `${s.diameter} mm`],
    ["Nozzle / bed", `${s.extruder_temp ?? "—"} / ${s.bed_temp ?? "—"} °C`],
    ["Price", s.price != null ? s.price.toFixed(2) : "—"],
    ["Lot", s.lot_nr || "—"],
    ["Last used", s.last_used ? new Date(s.last_used).toLocaleString() : "Never"],
    ["Abrasive", s.abrasive ? "Yes" : "No"],
  ];
  if (s.comment) facts.push(["Comment", s.comment]);
  return (
    <div className="spool-insp">
      <div className="spool-insp__head">
        <SpoolSwatch spool={s} size={44} />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div className="fil-eyebrow">Spool #{s.id}</div>
          <h2 className="spool-insp__name">{s.name}</h2>
          <div className="fil-sub" style={{ fontSize: 13 }}>
            {s.vendor ?? "No vendor"} · {materialLabel(s)}
          </div>
        </div>
        <button type="button" className="fil-close" aria-label="Close inspector" onClick={onClose}>
          ✕
        </button>
      </div>

      <div className="spool-insp__box">
        <div className="fil-bar fil-bar--big" aria-hidden="true">
          <div className={`fil-bar__fill${isLow ? " fil-bar__fill--low" : ""}`} style={{ width: `${Math.round(leftFraction(s) * 100)}%` }} />
        </div>
        <div className="spool-insp__split">
          <span className={isLow ? "fil-left--low" : undefined}>{grams(s.remaining_weight)} left</span>
          <span className="fil-sub">used {grams(s.used_weight)}</span>
          <span className="fil-sub">of {grams(s.initial_weight)}</span>
        </div>
      </div>

      <div className="spool-insp__actions">
        <button type="button" className="fil-btn fil-btn--primary" onClick={onLoad}>
          Load onto…
        </button>
        <button type="button" className="fil-btn" onClick={onEdit}>
          Edit
        </button>
        <button
          type="button"
          className="fil-btn"
          style={{ gridColumn: "1 / -1" }}
          onClick={() => void archive()}
        >
          {s.archived ? "Unarchive" : confirmArchive ? `Archive #${s.id}? Click again to confirm` : "Archive"}
        </button>
      </div>
      {row.loaded.length > 0 && !s.archived && confirmArchive && (
        <p className="fil-muted">It stays loaded on {row.loaded.map((l) => `${l.printer_name} ${l.label}`).join(", ")} until you unload it.</p>
      )}
      {archErr && <p className="fil-note fil-note--bad" role="alert">{archErr}</p>}

      <section aria-label="Weigh" className="spool-insp__box">
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <h3>Weigh · scale</h3>
          <span className="fil-sub">○ no scale yet</span>
        </div>
        <span className="fil-sub">
          Put the spool on a Teamster scale to update what is left. Devices arrive in a later session.
        </span>
        <button type="button" className="fil-btn" disabled>
          Use scale weight
        </button>
      </section>

      <div className="spool-insp__box spool-insp__colour">
        <SpoolSwatch spool={s} size={24} />
        <span style={{ flex: 1 }}>{colours}</span>
        <button type="button" className="fil-btn fil-btn--small" disabled title="TD1 colorimeter comes with devices">
          Measure
        </button>
      </div>

      <div className="spool-insp__box" style={{ gap: 2 }}>
        {facts.map(([k, v]) => (
          <div key={k} className="spool-insp__fact">
            <span>{k}</span>
            <span>{v}</span>
          </div>
        ))}
      </div>


    </div>
  );
}

// ── Edit (spool fields only; the filament form is next session) ───────────────

function EditSpoolPanel({ row, onClose, onSaved }: { row: SpoolRow; onClose: () => void; onSaved: () => void }) {
  const s = row.spool;
  const [location, setLocation] = useState(s.location ?? "");
  const [lot, setLot] = useState(s.lot_nr ?? "");
  const [comment, setComment] = useState(s.comment ?? "");
  const [price, setPrice] = useState(s.price != null ? String(s.price) : "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      const p = price.trim() === "" ? null : Number(price);
      if (p !== null && (!Number.isFinite(p) || p < 0)) throw new Error("Price must be a number, 0 or more.");
      await spools.edit(s.id, { location, lot_nr: lot, comment, price: p, archived: null });
      onSaved();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  };

  return (
    <div className="spool-insp">
      <div className="spool-insp__head">
        <SpoolSwatch spool={s} size={44} />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div className="fil-eyebrow">Edit spool #{s.id}</div>
          <h2 className="spool-insp__name">{s.name}</h2>
        </div>
        <button type="button" className="fil-close" aria-label="Close" onClick={onClose}>
          ✕
        </button>
      </div>
      <label className="fil-field">
        <span className="fil-field__label">Location</span>
        <input className="fil-input" value={location} onChange={(e) => setLocation(e.target.value)} placeholder="Shelf A1" />
      </label>
      <label className="fil-field">
        <span className="fil-field__label">Lot number</span>
        <input className="fil-input" value={lot} onChange={(e) => setLot(e.target.value)} />
      </label>
      <label className="fil-field">
        <span className="fil-field__label">Price</span>
        <input className="fil-input" inputMode="decimal" value={price} onChange={(e) => setPrice(e.target.value)} />
      </label>
      <label className="fil-field">
        <span className="fil-field__label">Comment</span>
        <input className="fil-input" value={comment} onChange={(e) => setComment(e.target.value)} />
      </label>
      <p className="fil-muted">Material, colour and vendor live on the filament; the filament form comes next session.</p>
      {error && <p className="fil-note fil-note--bad" role="alert">{error}</p>}
      <div style={{ flex: 1 }} />
      <div className="load-panel__foot">
        <button type="button" className="fil-btn fil-btn--primary" disabled={busy} onClick={() => void save()}>
          {busy ? "Saving…" : "Save spool"}
        </button>
        <button type="button" className="fil-btn" onClick={onClose}>
          Cancel
        </button>
      </div>
    </div>
  );
}
