/**
 * Load spool flow (W8, task 4.1): printer → tool / AMS slot → spool → guard → confirm.
 * One component, used from the printer page, the Fleet inspector and a spool row.
 * Reference: docs/ui-ref/Printer.dc.html lines 459–487 (load panel).
 */
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  grams,
  materialLabel,
  spools,
  whereLabel,
  type LoadCandidate,
  type LoadedSlot,
  type LoadResult,
  type GuardWarning,
} from "../api/spools";
import SpoolSwatch from "./SpoolSwatch";
import "./filament.css";

type Tone = "ok" | "warn" | "bad";

const SHORT: Record<GuardWarning["code"], string> = {
  material: "Material",
  abrasive: "Abrasive",
  low_remaining: "Low",
  tray_mismatch: "Tray differs",
  loaded_elsewhere: "In use",
  archived: "Archived",
};

function tagOf(ws: GuardWarning[]): { tone: Tone; text: string } {
  const block = ws.find((w) => w.severity === "block");
  if (block) return { tone: "bad", text: `✖ ${SHORT[block.code]}` };
  if (ws.length) return { tone: "warn", text: `⚠ ${SHORT[ws[0].code]}` };
  return { tone: "ok", text: "✓ OK" };
}

function matches(c: LoadCandidate, q: string): boolean {
  const s = c.spool;
  const hay = [
    `#${s.id}`,
    s.name,
    s.vendor,
    s.material,
    s.modifier,
    s.location,
    s.color_hex,
    ...s.multi_color_hexes,
    ...c.loaded.map((l) => l.printer_name),
  ]
    .filter(Boolean)
    .join(" ")
    .toLowerCase();
  return q
    .toLowerCase()
    .replace(/#(?=[0-9a-f]{6})/g, "")
    .split(/\s+/)
    .filter(Boolean)
    .every((w) => hay.includes(w));
}

function slotLine(s: LoadedSlot): string {
  if (s.spool) return `#${s.spool.id} ${s.spool.name}`;
  if (s.spool_id != null) return `Spool #${s.spool_id} (not in Spoolman)`;
  if (s.tray) return `${s.tray.material ?? "?"}${s.tray.remaining != null ? ` · ${s.tray.remaining}%` : ""}`;
  return "Empty";
}

export default function LoadSpoolPanel({
  printerId,
  tool,
  spoolId,
  onClose,
  onLoaded,
}: {
  /** Fixed printer (printer page, Fleet). Omit to choose one (spool row). */
  printerId?: number;
  tool?: number;
  /** Preselected spool (spool row). */
  spoolId?: number;
  onClose: () => void;
  onLoaded?: (r: LoadResult) => void;
}) {
  const [slots, setSlots] = useState<LoadedSlot[] | null>(null);
  const [pid, setPid] = useState<number | null>(printerId ?? null);
  const [toolSel, setToolSel] = useState<number | null>(tool ?? null);
  const [cands, setCands] = useState<LoadCandidate[] | null>(null);
  const [q, setQ] = useState("");
  const [pick, setPick] = useState<number | null>(spoolId ?? null);
  const [override, setOverride] = useState(false);
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<string | null>(null);

  const loadSlots = useCallback(async () => {
    try {
      setSlots(await spools.loaded());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);
  useEffect(() => void loadSlots(), [loadSlots]);

  const printers = useMemo(() => {
    const m = new Map<number, { id: number; name: string; writable: boolean }>();
    for (const s of slots ?? []) {
      const p = m.get(s.printer_id) ?? { id: s.printer_id, name: s.printer_name, writable: false };
      p.writable ||= s.writable;
      m.set(s.printer_id, p);
    }
    return [...m.values()];
  }, [slots]);

  // Spool row entry: preselect the first printer that can take a spool.
  useEffect(() => {
    if (pid == null && printers.length) setPid((printers.find((p) => p.writable) ?? printers[0]).id);
  }, [pid, printers]);

  const pSlots = useMemo(() => (slots ?? []).filter((s) => s.printer_id === pid), [slots, pid]);
  useEffect(() => {
    if (toolSel == null && pSlots.length) setToolSel((pSlots.find((s) => s.writable) ?? pSlots[0]).tool);
  }, [toolSel, pSlots]);
  const slot = pSlots.find((s) => s.tool === toolSel) ?? null;

  useEffect(() => {
    setCands(null);
    if (pid == null || toolSel == null) return;
    let live = true;
    spools
      .candidates(pid, toolSel)
      .then((c) => live && setCands(c))
      .catch((e) => live && setError(e instanceof Error ? e.message : String(e)));
    return () => {
      live = false;
    };
  }, [pid, toolSel, done]);

  const shown = useMemo(() => (cands ?? []).filter((c) => matches(c, q)), [cands, q]);
  const picked = (cands ?? []).find((c) => c.spool.id === pick) ?? null;
  const blocked = !!picked?.blocked;
  const pickedHere = pick != null && slot?.spool_id === pick;
  const canLoad =
    !!slot?.writable && !!picked && !busy && (!blocked || (override && reason.trim().length > 0));

  const choosePrinter = (id: number) => {
    setPid(id);
    setToolSel(null);
    setOverride(false);
    setError(null);
  };

  const doLoad = async () => {
    if (!slot || !picked || pid == null) return;
    setBusy(true);
    setError(null);
    try {
      const r = await spools.load(pid, slot.tool, picked.spool.id, blocked ? reason.trim() : undefined);
      const moved = r.unloaded_from.length
        ? ` Unloaded from ${r.unloaded_from.map((u) => `${u.printer_name} ${u.label}`).join(", ")}.`
        : "";
      setDone(`Loaded #${picked.spool.id} ${picked.spool.name} on ${slot.printer_name} ${slot.label}.${moved}`);
      setOverride(false);
      setReason("");
      setPick(null);
      await loadSlots();
      onLoaded?.(r);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const doUnload = async () => {
    if (!slot || pid == null) return;
    setBusy(true);
    setError(null);
    try {
      await spools.unload(pid, slot.tool);
      setDone(`Unloaded ${slot.printer_name} ${slot.label}.`);
      await loadSlots();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="load-panel">
      <div className="load-panel__head">
        <div style={{ flex: 1, minWidth: 0 }}>
          <div className="fil-eyebrow">Load spool</div>
          <h2 className="load-panel__title">
            {slot ? `${slot.printer_name} · ${slot.label}` : "Choose a printer"}
          </h2>
        </div>
        <button type="button" className="fil-close" aria-label="Close" onClick={onClose}>
          ✕
        </button>
      </div>

      {printerId == null && printers.length > 1 && (
        <label className="fil-field">
          <span className="fil-field__label">Printer</span>
          <select
            className="fil-input"
            value={pid ?? ""}
            onChange={(e) => choosePrinter(Number(e.target.value))}
          >
            {printers.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
                {p.writable ? "" : " (read-only)"}
              </option>
            ))}
          </select>
        </label>
      )}

      {pSlots.length > 1 || (pSlots.length === 1 && !pSlots[0].writable) ? (
        <div role="radiogroup" aria-label="Tool or slot" className="load-panel__slots">
          {pSlots.map((s) => (
            <button
              key={s.tool}
              type="button"
              role="radio"
              aria-checked={s.tool === toolSel}
              className={`load-slot${s.tool === toolSel ? " load-slot--on" : ""}`}
              onClick={() => {
                setToolSel(s.tool);
                setOverride(false);
              }}
            >
              <span className="load-slot__label">{s.label}</span>
              <span className="load-slot__row">
                <SpoolSwatch spool={s.spool} hex={s.tray?.color} size={18} />
                <span>{slotLine(s)}</span>
              </span>
            </button>
          ))}
        </div>
      ) : null}

      {slot && !slot.writable && (
        <p className="fil-note fil-note--info">{slot.read_only_reason}</p>
      )}

      {slot?.writable && (
        <>
          <div className="load-panel__current">
            <SpoolSwatch spool={slot.spool} size={22} />
            <span style={{ flex: 1 }}>
              Now: <b>{slotLine(slot)}</b>
            </span>
            {slot.spool_id != null && (
              <button type="button" className="fil-btn fil-btn--small" onClick={() => void doUnload()} disabled={busy}>
                Unload
              </button>
            )}
          </div>

          <label className="fil-field">
            <span className="fil-field__label">Find spool</span>
            <input
              type="search"
              className="fil-input"
              placeholder="#id, name, material, colour, vendor or location"
              value={q}
              onChange={(e) => setQ(e.target.value)}
            />
          </label>

          {cands == null ? (
            <p className="fil-muted">Checking spools…</p>
          ) : shown.length === 0 ? (
            <p className="fil-muted">{cands.length ? "No spool matches." : "No other spools in Spoolman."}</p>
          ) : (
            <div role="radiogroup" aria-label="Spools" className="load-panel__cands">
              {shown.map((c) => {
                const on = c.spool.id === pick;
                const t = tagOf(c.warnings);
                return (
                  <button
                    key={c.spool.id}
                    type="button"
                    role="radio"
                    aria-checked={on}
                    className={`load-cand${on ? " load-cand--on" : ""}`}
                    onClick={() => {
                      setPick(c.spool.id);
                      setOverride(false);
                      setDone(null);
                    }}
                  >
                    <span className="load-cand__row">
                      <SpoolSwatch spool={c.spool} size={22} />
                      <b className="load-cand__name">
                        #{c.spool.id} {c.spool.name}
                      </b>
                      <span className={`fil-tag fil-tag--${t.tone}`}>{t.text}</span>
                    </span>
                    <span className="load-cand__sub">
                      {materialLabel(c.spool)} · {grams(c.spool.remaining_weight)} ·{" "}
                      {c.loaded.length ? `loaded on ${c.loaded.map((l) => l.printer_name).join(", ")}` : whereLabel(c.spool, [])}
                      {c.spool.vendor ? ` · ${c.spool.vendor}` : ""}
                    </span>
                    {on &&
                      c.warnings.map((w, i) => (
                        <span key={i} className={`load-cand__why load-cand__why--${w.severity}`}>
                          {w.reason}
                        </span>
                      ))}
                  </button>
                );
              })}
            </div>
          )}
          {pickedHere && <p className="fil-muted">That spool is already loaded here.</p>}

          {blocked && (
            <div className="load-override">
              <label className="load-override__check">
                <input type="checkbox" checked={override} onChange={(e) => setOverride(e.target.checked)} />
                <span>Load anyway. I know why the guard blocks this.</span>
              </label>
              {override && (
                <label className="fil-field">
                  <span className="fil-field__label">Reason (saved in the audit log)</span>
                  <input
                    className="fil-input"
                    value={reason}
                    onChange={(e) => setReason(e.target.value)}
                    placeholder="e.g. hardened nozzle fitted today"
                  />
                </label>
              )}
            </div>
          )}
        </>
      )}

      {error && <p className="fil-note fil-note--bad" role="alert">{error}</p>}
      {done && <p className="fil-note fil-note--ok" role="status">{done}</p>}

      <div style={{ flex: 1 }} />
      <div className="load-panel__foot">
        <button type="button" className="fil-btn fil-btn--primary" disabled={!canLoad} onClick={() => void doLoad()}>
          {busy ? "Loading…" : picked && slot ? `Load #${picked.spool.id} into ${slot.label}` : "Load"}
        </button>
        <button type="button" className="fil-btn" onClick={onClose}>
          {done ? "Close" : "Cancel"}
        </button>
      </div>
    </div>
  );
}
