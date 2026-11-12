/**
 * Filament page — Spools, Filaments, Inventory, Vendors, Locations (tasks 4.1–4.3).
 * Reference: docs/ui-ref/Filament.dc.html lines 65–147 (lists), 148–195 (inventory),
 * 196–208 (vendors), 222–260 (spool inspector), 261–420 (filament + vendor forms),
 * 443–554 (new spool).
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
import SpoolSwatch, { ColourSwatch } from "../filament/SpoolSwatch";
import LoadSpoolPanel from "../filament/LoadSpoolPanel";
import FilamentPanel from "../filament/FilamentForm";
import NewSpoolPanel from "../filament/NewSpoolPanel";
import VendorPanel from "../filament/VendorPanel";
import {
  COLOUR_KINDS,
  filamentApi,
  materialFull,
  type FilamentRow,
  type MaterialCatalogue,
  type VendorRow,
} from "../api/filaments";
import "../filament/filament.css";
import { ExternalLink } from "../ui";
import InventoryTab from "../filament/InventoryTab";
import LocationsTab, { NO_LOCATION } from "../filament/LocationsTab";
import { inventoryApi, type Inventory, type Locations } from "../api/inventory";

const TABS = [
  ["spools", "Spools"],
  ["filaments", "Filaments"],
  ["inventory", "Inventory"],
  ["vendors", "Vendors"],
  ["locations", "Locations"],
] as const;
type Tab = (typeof TABS)[number][0];

/** What the inspector shows. */
type Panel =
  | { kind: "spool"; id: number }
  | { kind: "load"; id: number }
  | { kind: "edit"; id: number }
  | { kind: "new-spool"; filamentId: number | null; qty?: number }
  | { kind: "filament"; id: number | null; vendor?: string }
  | { kind: "vendor"; id: number | null };

export default function Filament() {
  const { openInspector, closeInspector, inspectorOpen, inspectorBodyId } =
    useOutletContext<ShellContext>();
  const [tab, setTab] = useState<Tab>("spools");
  const [rows, setRows] = useState<SpoolRow[] | null>(null);
  const [fils, setFils] = useState<FilamentRow[] | null>(null);
  const [vendors, setVendors] = useState<VendorRow[]>([]);
  const [locations, setLocations] = useState<string[]>([]);
  const [inv, setInv] = useState<Inventory | null>(null);
  const [locs, setLocs] = useState<Locations | null>(null);
  const [loc, setLoc] = useState("");
  const [catalogue, setCatalogue] = useState<MaterialCatalogue | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [q, setQ] = useState("");
  const [fq, setFq] = useState("");
  const [mat, setMat] = useState("all");
  const [low, setLow] = useState(false);
  const [showArchived, setShowArchived] = useState(false);
  const [panel, setPanel] = useState<Panel | null>(null);
  const [toast, setToast] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const [sp, fl, vd, lc, iv, ls] = await Promise.all([
        spools.list(showArchived),
        filamentApi.filaments(),
        filamentApi.vendors(),
        filamentApi.locations(),
        inventoryApi.get(),
        inventoryApi.locations(),
      ]);
      setRows(sp);
      setFils(fl);
      setVendors(vd);
      setLocations(lc);
      setInv(iv);
      setLocs(ls);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [showArchived]);
  useEffect(() => void load(), [load]);
  useEffect(() => {
    filamentApi.catalogue().then(setCatalogue, (e) => setError(e instanceof Error ? e.message : String(e)));
  }, []);
  const onSpoolEvent = useCallback(() => void load(), [load]);
  useEventStream(onSpoolEvent, SPOOL_TOPICS);
  useEffect(() => () => closeInspector(), [closeInspector]);
  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), 5000);
    return () => clearTimeout(t);
  }, [toast]);

  const [portal, setPortal] = useState<Element | null>(null);
  useEffect(() => setPortal(document.getElementById(inspectorBodyId)), [inspectorBodyId, inspectorOpen]);

  const all = rows ?? [];
  const filaments = fils ?? [];
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
    if (loc === NO_LOCATION ? !!s.location?.trim() : loc && s.location !== loc) return false;
    if (!q.trim()) return true;
    const hay = [`#${s.id}`, s.name, s.vendor, s.material, s.modifier, s.location, s.color_hex, s.lot_nr, ...r.loaded.map((l) => l.printer_name)]
      .filter(Boolean)
      .join(" ")
      .toLowerCase();
    return q.toLowerCase().replace(/#(?=[0-9a-f]{6})/g, "").split(/\s+/).filter(Boolean).every((w) => hay.includes(w));
  });
  const filsShown = filaments.filter((f) => {
    if (!fq.trim()) return true;
    const hay = [f.vendor, f.name, f.material, ...f.modifiers, ...f.colours].filter(Boolean).join(" ").toLowerCase();
    return fq.toLowerCase().replace(/#/g, "").split(/\s+/).filter(Boolean).every((w) => hay.includes(w));
  });

  const kg = all.reduce((a, r) => a + (r.spool.remaining_weight ?? 0), 0) / 1000;
  const lowCount = all.filter((r) => r.spool.remaining_weight != null && r.spool.remaining_weight < LOW_GRAMS).length;
  const spoolId = panel && (panel.kind === "spool" || panel.kind === "load" || panel.kind === "edit") ? panel.id : null;
  const cur = all.find((r) => r.spool.id === spoolId) ?? null;
  const curFil = panel?.kind === "filament" && panel.id != null ? filaments.find((f) => f.id === panel.id) ?? null : null;
  const curVendor = panel?.kind === "vendor" && panel.id != null ? vendors.find((v) => v.id === panel.id) ?? null : null;

  const show = (p: Panel) => {
    setPanel(p);
    openInspector();
  };
  const close = () => {
    setPanel(null);
    closeInspector();
  };

  return (
    <div className="fil-page">
      <header className="fil-head">
        <div>
          <h1 className="fil-head__title phone-dup-title">Filament</h1>
          <p className="fil-head__sub">
            {rows == null
              ? "Loading spools…"
              : `${all.length} spool${all.length === 1 ? "" : "s"} · ${kg.toFixed(1)} kg on hand · ${lowCount} low · ${filaments.length} filament${filaments.length === 1 ? "" : "s"} · ${vendors.length} vendor${vendors.length === 1 ? "" : "s"}`}
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
        {toast && <p className="fil-note fil-note--ok" role="status">{toast}</p>}
        {tab === "filaments" ? (
          <FilamentsTab
            rows={filsShown}
            total={filaments.length}
            q={fq}
            setQ={setFq}
            selected={panel?.kind === "filament" ? panel.id : null}
            onOpen={(f) => show({ kind: "filament", id: f.id })}
            onAdd={() => show({ kind: "filament", id: null })}
          />
        ) : tab === "vendors" ? (
          <VendorsTab
            rows={vendors}
            selected={panel?.kind === "vendor" ? panel.id : null}
            onOpen={(v) => show({ kind: "vendor", id: v.id })}
            onAdd={() => show({ kind: "vendor", id: null })}
          />
        ) : tab === "inventory" ? (
          <InventoryTab
            inv={inv}
            filaments={filaments}
            vendors={vendors}
            onChange={setInv}
            onToast={setToast}
            onBought={(r) => show({ kind: "new-spool", filamentId: r.filament.id, qty: r.buy })}
          />
        ) : tab === "locations" ? (
          <LocationsTab
            data={locs}
            onShow={(l) => {
              setLoc(l);
              setTab("spools");
            }}
            onRenamed={(m) => {
              setToast(m);
              setLoc("");
              void load();
            }}
          />
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
              <button type="button" className="fil-btn fil-btn--primary fil-btn--add" onClick={() => show({ kind: "new-spool", filamentId: null })}>
                Add spool
              </button>
            </div>
            <div className="fil-filters">
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
              <select className="fil-input fil-select" aria-label="Location" value={loc} onChange={(e) => setLoc(e.target.value)}>
                <option value="">All locations</option>
                {(locs?.locations ?? []).map((l) => (
                  <option key={l.name} value={l.name}>
                    {l.name} · {l.spool_count}
                  </option>
                ))}
                {loc && loc !== NO_LOCATION && !locs?.locations.some((l) => l.name === loc) && <option value={loc}>{loc}</option>}
                <option value={NO_LOCATION}>No location</option>
              </select>
            </div>
            <div className="fil-count">{shown.length} shown</div>
            <div className="fil-table-wrap">
              <table className="fil-table ph-table ph-grid">
                <thead>
                  <tr>
                    <th>Spool</th>
                    <th>Material</th>
                    <th style={{ width: 220 }}>Left</th>
                    <th className="ph-hide">Where</th>
                  </tr>
                </thead>
                <tbody>
                  {shown.map((r) => {
                    const s = r.spool;
                    const isLow = s.remaining_weight != null && s.remaining_weight < LOW_GRAMS;
                    return (
                      <tr key={s.id} aria-selected={spoolId === s.id} className="fil-tr" onClick={() => show({ kind: "spool", id: s.id })}>
                        <td>
                          <button
                            type="button"
                            className="fil-row-btn"
                            onClick={(e) => {
                              e.stopPropagation();
                              show({ kind: "spool", id: s.id });
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
                        <td className="ph-hide">{whereLabel(s, r.loaded)}</td>
                      </tr>
                    );
                  })}
                  {rows != null && shown.length === 0 && (
                    <tr>
                      <td colSpan={4} style={{ padding: "var(--fw-space-4)" }} className="fil-placeholder">
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
        panel &&
        createPortal(
          panel.kind === "load" && cur ? (
            <LoadSpoolPanel spoolId={cur.spool.id} onClose={() => show({ kind: "spool", id: cur.spool.id })} onLoaded={() => void load()} />
          ) : panel.kind === "edit" && cur ? (
            <EditSpoolPanel
              row={cur}
              onClose={() => show({ kind: "spool", id: cur.spool.id })}
              onSaved={() => void load().then(() => show({ kind: "spool", id: cur.spool.id }))}
            />
          ) : panel.kind === "spool" && cur ? (
            <SpoolInspector
              row={cur}
              onClose={close}
              onLoad={() => show({ kind: "load", id: cur.spool.id })}
              onEdit={() => show({ kind: "edit", id: cur.spool.id })}
              onEditFilament={() => show({ kind: "filament", id: cur.spool.filament_id })}
              onClone={async (n) => {
                const made = await filamentApi.cloneSpool(cur.spool.id, n);
                setToast(`Cloned #${cur.spool.id}: ${made.map((m) => `#${m.id}`).join(", ")}`);
                await load();
              }}
              onArchive={async (archived) => {
                await spools.edit(cur.spool.id, { archived, location: null, lot_nr: null, comment: null, price: null });
                if (archived && !showArchived) close();
                await load();
              }}
            />
          ) : panel.kind === "new-spool" ? (
            <NewSpoolPanel
              key={`new-${panel.filamentId ?? "x"}-${panel.qty ?? 1}`}
              catalogue={catalogue}
              vendors={vendors}
              filaments={filaments}
              locations={locations}
              preset={filaments.find((f) => f.id === panel.filamentId) ?? null}
              presetQty={panel.qty}
              onClose={close}
              onCreated={(c) => {
                setToast(`Added ${c.spools.length} spool${c.spools.length === 1 ? "" : "s"} of ${[c.filament.vendor, c.filament.name].filter(Boolean).join(" ")}`);
                void load();
              }}
              onLoad={(id) => {
                setTab("spools");
                void load().then(() => show({ kind: "load", id }));
              }}
            />
          ) : panel.kind === "filament" && (panel.id == null || curFil) ? (
            <FilamentPanel
              key={`fil-${panel.id ?? "new"}`}
              filament={curFil}
              catalogue={catalogue}
              vendors={vendors}
              filaments={filaments}
              presetVendor={panel.vendor}
              onClose={close}
              onOpenDuplicate={(f) => show({ kind: "filament", id: f.id })}
              onSaved={(f, addSpool) => {
                setToast(`Saved filament ${[f.vendor, f.name].filter(Boolean).join(" ")}`);
                void load().then(() => (addSpool ? show({ kind: "new-spool", filamentId: f.id }) : show({ kind: "filament", id: f.id })));
              }}
            />
          ) : panel.kind === "vendor" && (panel.id == null || curVendor) ? (
            <VendorPanel
              key={`vend-${panel.id ?? "new"}`}
              vendor={curVendor}
              vendors={vendors}
              onClose={close}
              onSaved={(v, addFilament) => {
                setToast(`Saved vendor ${v.name}`);
                void load().then(() =>
                  addFilament ? show({ kind: "filament", id: null, vendor: v.name }) : show({ kind: "vendor", id: v.id }),
                );
              }}
            />
          ) : null,
          portal,
        )}
    </div>
  );
}

// ── Filaments tab (reference lines 122–147) ──────────────────────────────────

function FilamentsTab({
  rows,
  total,
  q,
  setQ,
  selected,
  onOpen,
  onAdd,
}: {
  rows: FilamentRow[];
  total: number;
  q: string;
  setQ: (q: string) => void;
  selected: number | null;
  onOpen: (f: FilamentRow) => void;
  onAdd: () => void;
}) {
  return (
    <>
      <div className="fil-filters">
        <input
          type="search"
          className="fil-input"
          aria-label="Search filaments"
          placeholder="Name, vendor, material, colour…"
          value={q}
          onChange={(e) => setQ(e.target.value)}
        />
        <button type="button" className="fil-btn fil-btn--primary fil-btn--add" onClick={onAdd}>
          Add filament
        </button>
      </div>
      <div className="fil-count">
        {rows.length === total ? `${total} filaments` : `${rows.length} of ${total} filaments`}
      </div>
      <div className="fil-table-wrap">
        <table className="fil-table ph-table ph-grid">
          <thead>
            <tr>
              <th>Filament</th>
              <th>Material</th>
              <th className="ph-hide">Nozzle / bed</th>
              <th className="ph-hide">TD</th>
              <th>Stock</th>
              <th className="ph-hide">Extras</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((f) => (
              <tr key={f.id} className="fil-tr" aria-selected={selected === f.id} onClick={() => onOpen(f)}>
                <td>
                  <button
                    type="button"
                    className="fil-row-btn"
                    onClick={(e) => {
                      e.stopPropagation();
                      onOpen(f);
                    }}
                  >
                    <ColourSwatch colours={f.colours} kind={f.colour_kind} size={28} />
                    <span>
                      <b>{f.name}</b>
                      <br />
                      <span className="fil-sub">{f.vendor ?? "No vendor"}</span>
                    </span>
                  </button>
                </td>
                <td>
                  <span className="fil-mono">{materialFull(f.material ?? "—", f.modifiers)}</span>
                  <br />
                  <span className="fil-sub">
                    {COLOUR_KINDS.find((k) => k[0] === f.colour_kind)?.[1]}
                    {f.colours.length > 1 ? ` · ${f.colours.length} colours` : ""}
                  </span>
                </td>
                <td className="fil-mono ph-hide">
                  {f.extruder_temp ?? "—"} / {f.bed_temp ?? "—"} °C
                </td>
                <td className="fil-mono ph-hide">{f.hue_td ?? "—"}</td>
                <td>
                  {f.spool_count} spool{f.spool_count === 1 ? "" : "s"}
                  {f.spool_count > 0 ? ` · ${Math.round(f.grams_left)} g` : ""}
                </td>
                <td className="fil-sub ph-hide">
                  <span className="fil-extras">
                    {f.abrasive && <span className="fil-abrasive">⚠ abrasive</span>}
                    {f.url && (
                      <ExternalLink href={f.url} label={`Product page for ${f.name}`}>
                        Link
                      </ExternalLink>
                    )}
                    {f.price != null && <span>{f.price.toFixed(2)}</span>}
                    {!f.abrasive && !f.url && f.price == null && "—"}
                  </span>
                </td>
              </tr>
            ))}
            {rows.length === 0 && (
              <tr>
                <td colSpan={6} style={{ padding: "var(--fw-space-4)" }} className="fil-placeholder">
                  {total ? "No filament matches." : "No filaments yet. Add one, or add a spool and create its filament there."}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </>
  );
}

// ── Vendors tab (reference lines 196–208) ────────────────────────────────────

function VendorsTab({
  rows,
  selected,
  onOpen,
  onAdd,
}: {
  rows: VendorRow[];
  selected: number | null;
  onOpen: (v: VendorRow) => void;
  onAdd: () => void;
}) {
  return (
    <>
      <div className="fil-filters">
        <div className="fil-count" style={{ flex: 1 }}>
          {rows.length} vendor{rows.length === 1 ? "" : "s"}
        </div>
        <button type="button" className="fil-btn fil-btn--primary fil-btn--add" onClick={onAdd}>
          Add vendor
        </button>
      </div>
      <div className="fil-table-wrap">
        <table className="fil-table ph-table ph-grid">
          <thead>
            <tr>
              <th>Vendor</th>
              <th className="ph-hide">Empty spool</th>
              <th>Filaments</th>
              <th className="ph-hide">Website</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((v) => (
              <tr key={v.id} className="fil-tr" aria-selected={selected === v.id} onClick={() => onOpen(v)}>
                <td>
                  <button
                    type="button"
                    className="fil-row-btn"
                    onClick={(e) => {
                      e.stopPropagation();
                      onOpen(v);
                    }}
                  >
                    <span>
                      <b>{v.name}</b>
                      {v.comment && (
                        <>
                          <br />
                          <span className="fil-sub">{v.comment}</span>
                        </>
                      )}
                    </span>
                  </button>
                </td>
                <td className="fil-mono ph-hide">{v.empty_spool_weight != null ? `${v.empty_spool_weight} g` : "—"}</td>
                <td>{v.filament_count}</td>
                <td className="fil-sub ph-hide">{v.website ? <ExternalLink href={v.website} label={`${v.name} website`} /> : "—"}</td>
              </tr>
            ))}
            {rows.length === 0 && (
              <tr>
                <td colSpan={4} style={{ padding: "var(--fw-space-4)" }} className="fil-placeholder">
                  No vendors yet.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </>
  );
}

// ── Spool inspector ───────────────────────────────────────────────────────────

function SpoolInspector({
  row,
  onClose,
  onLoad,
  onEdit,
  onEditFilament,
  onClone,
  onArchive,
}: {
  row: SpoolRow;
  onClose: () => void;
  onLoad: () => void;
  onEdit: () => void;
  onEditFilament: () => void;
  onClone: (n: number) => Promise<void>;
  onArchive: (archived: boolean) => Promise<void>;
}) {
  const s = row.spool;
  const [confirmArchive, setConfirmArchive] = useState(false);
  const [archErr, setArchErr] = useState<string | null>(null);
  const [cloneN, setCloneN] = useState<number | null>(null);
  const [cloneBusy, setCloneBusy] = useState(false);
  useEffect(() => {
    setConfirmArchive(false);
    setCloneN(null);
  }, [s.id]);
  const clone = async () => {
    if (cloneN == null) return;
    setCloneBusy(true);
    setArchErr(null);
    try {
      await onClone(cloneN);
      setCloneN(null);
    } catch (e) {
      setArchErr(e instanceof Error ? e.message : String(e));
    } finally {
      setCloneBusy(false);
    }
  };
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
          Edit spool
        </button>
        <button type="button" className="fil-btn" onClick={onEditFilament}>
          Edit filament
        </button>
        <button type="button" className="fil-btn" aria-expanded={cloneN != null} onClick={() => setCloneN(cloneN == null ? 1 : null)}>
          Clone…
        </button>
        {cloneN != null && (
          <div className="spool-insp__clone" style={{ gridColumn: "1 / -1" }}>
            <span className="fil-sub">Copies (same filament, weights, location, lot; full):</span>
            <div className="ns-qty__ctl">
              <button type="button" className="fil-btn fil-btn--small" aria-label="Fewer" disabled={cloneN <= 1} onClick={() => setCloneN(cloneN - 1)}>
                −
              </button>
              <output className="ns-qty__n">{cloneN}</output>
              <button type="button" className="fil-btn fil-btn--small" aria-label="More" disabled={cloneN >= 50} onClick={() => setCloneN(cloneN + 1)}>
                +
              </button>
            </div>
            <button type="button" className="fil-btn fil-btn--small" disabled={cloneBusy} onClick={() => void clone()}>
              {cloneBusy ? "Cloning…" : `Clone ${cloneN}×`}
            </button>
          </div>
        )}
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
        <div style={{ display: "flex", alignItems: "center", gap: "var(--fw-space-2)" }}>
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

// ── Edit (spool fields only; the filament has its own form) ──────────────────

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
      <p className="fil-muted">Material, colour and vendor live on the filament: use Edit filament.</p>
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
