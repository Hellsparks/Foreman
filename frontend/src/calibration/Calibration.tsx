/**
 * Tools → Calibration (task 5.4, W10). Two flows that never mix:
 * skew belongs to a printer (Klipper), shrinkage to a filament.
 * Nothing here talks to a printer unless the owner presses a button that says so.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  calibrationApi,
  type MacroPatch,
  type ShrinkageResult,
  type ShrinkFilament,
  type SkewPreview,
  type SkewPrinter,
  type SkewRecord,
} from "../api/calibration";
import { useEventStream } from "../hooks/useEventStream";
import { ToastStack, useToasts } from "../ui";
import {
  axesFrom,
  type Axis,
  type AxisFields,
  deg,
  EMPTY_AXES,
  EMPTY_PLANES,
  pct,
  type PlaneFields,
  planesFrom,
  shortDate,
  shrinkInput,
  skewSummary,
} from "./calLogic";

type Mode = "skew" | "shr";
const TOPICS = ["printer.status", "printer.connection"];

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export default function Calibration() {
  const [mode, setMode] = useState<Mode>("skew");
  const [printers, setPrinters] = useState<SkewPrinter[]>([]);
  const [filaments, setFilaments] = useState<ShrinkFilament[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [skewP, setSkewP] = useState<number | null>(null);
  const { toasts, addToast, removeToast } = useToasts();

  const load = useCallback(async () => {
    try {
      const p = await calibrationApi.skewPrinters();
      setPrinters(p);
      setSkewP((cur) => cur ?? p.find((x) => x.supported)?.id ?? p[0]?.id ?? null);
      setError(null);
    } catch (e) {
      setError(errText(e));
    }
    try {
      setFilaments(await calibrationApi.shrinkFilaments());
    } catch (e) {
      setFilaments([]);
      setError(errText(e));
    }
  }, []);
  useEffect(() => void load(), [load]);
  // Printer state changes (idle ↔ printing) decide whether Apply is allowed.
  const lastRefresh = useRef(0);
  const onEvent = useCallback(() => {
    const now = Date.now();
    if (now - lastRefresh.current < 5000) return;
    lastRefresh.current = now;
    void calibrationApi.skewPrinters().then(setPrinters, () => {});
  }, []);
  useEventStream(onEvent, TOPICS);

  const klipper = printers.filter((p) => p.supported);
  const corrected = klipper.filter((p) => p.applied).length;
  const calibrated = (filaments ?? []).filter((f) => f.shrinkage_xy != null || f.shrinkage_z != null).length;

  const cards: { id: Mode; kicker: string; title: string; desc: string; status: string; tone: string }[] = [
    {
      id: "skew",
      kicker: "Per printer · do first",
      title: "Printer skew",
      desc: "Frame not square. Klipper [skew_correction], stored on the printer.",
      status: `${corrected} of ${klipper.length} Klipper printer${klipper.length === 1 ? "" : "s"} corrected`,
      tone: corrected < klipper.length ? "warn" : "ok",
    },
    {
      id: "shr",
      kicker: "Per filament",
      title: "Filament shrinkage",
      desc: "Material shrinks as it cools. Stored on the filament, used by Orca export.",
      status: filaments ? `${calibrated} of ${filaments.length} filaments calibrated` : "…",
      tone: "muted",
    },
  ];

  return (
    <div className="tl-cal">
      <div role="tablist" aria-label="Calibration type" className="tl-modes">
        {cards.map((c) => (
          <button key={c.id} type="button" role="tab" aria-selected={mode === c.id} className="tl-mode" onClick={() => setMode(c.id)}>
            <span className="tl-kicker">{c.kicker}</span>
            <b className="tl-mode__title">{c.title}</b>
            <span className="tl-mode__desc">{c.desc}</span>
            <span className={`tl-mono tl-fg--${c.tone}`}>{c.status}</span>
          </button>
        ))}
      </div>
      <p className="tl-muted tl-small">
        Order matters: skew is a frame error, so fix it once per printer first. Shrinkage is a material property, stored per
        filament and measured on a printer that is already square, or the skew error leaks into the shrinkage number.
      </p>
      {error && (
        <p className="tl-error" role="alert">
          {error}
        </p>
      )}
      {mode === "skew" ? (
        <SkewFlow printers={printers} selected={skewP} onSelect={setSkewP} onChanged={load} toast={addToast} onError={setError} />
      ) : (
        <ShrinkFlow
          printers={printers}
          filaments={filaments ?? []}
          onChanged={load}
          toast={addToast}
          onError={setError}
          goSkew={(id) => {
            setSkewP(id);
            setMode("skew");
          }}
        />
      )}
      <ToastStack toasts={toasts} onDismiss={removeToast} />
    </div>
  );
}

// ---------------------------------------------------------------------------
// Skew
// ---------------------------------------------------------------------------

type Toast = (text: string, undo?: () => void) => void;

function stateLabel(p: SkewPrinter): string {
  return p.state ?? "offline";
}

function printerOption(p: SkewPrinter): string {
  if (!p.supported) return `${p.name} · not supported`;
  if (p.applied) return `${p.name} · corrected ${shortDate(p.applied.applied_at ?? p.applied.measured_at)}`;
  return `${p.name} · not calibrated`;
}

function SkewFlow({ printers, selected, onSelect, onChanged, toast, onError }: {
  printers: SkewPrinter[];
  selected: number | null;
  onSelect: (id: number) => void;
  onChanged: () => Promise<void>;
  toast: Toast;
  onError: (e: string | null) => void;
}) {
  const p = printers.find((x) => x.id === selected) ?? null;
  const [fields, setFields] = useState<PlaneFields>(EMPTY_PLANES);
  const [more, setMore] = useState(false);
  const [preview, setPreview] = useState<SkewPreview | null>(null);
  const [previewErr, setPreviewErr] = useState<string | null>(null);
  const [ack, setAck] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    setAck(false);
  }, [selected]);

  const planes = useMemo(() => planesFrom(fields, more), [fields, more]);
  useEffect(() => {
    if (!planes) {
      setPreview(null);
      setPreviewErr(null);
      return;
    }
    const t = setTimeout(() => {
      calibrationApi.skewPreview(planes).then(
        (r) => {
          setPreview(r);
          setPreviewErr(null);
        },
        (e) => {
          setPreview(null);
          setPreviewErr(errText(e));
        },
      );
    }, 200);
    return () => clearTimeout(t);
  }, [planes]);

  if (printers.length === 0) return <p className="tl-muted">No printers yet.</p>;
  const idle = p?.state === "idle";
  const sendBlock = !p?.supported
    ? "Not a Klipper printer."
    : p.state === "printing" || p.state === "paused"
      ? `${p.name} is ${p.state} — refused until the job ends.`
      : !idle
        ? `${p.name} is ${stateLabel(p)} — Foreman only sends this to an idle printer.`
        : null;

  const remeasure = (r: SkewRecord) => {
    const f: PlaneFields = { ...EMPTY_PLANES };
    for (const pl of r.planes) f[pl.plane] = [String(pl.ac), String(pl.bd), String(pl.ad)];
    setFields(f);
    setMore(r.planes.some((pl) => pl.plane !== "XY"));
  };

  const clear = async () => {
    if (!p) return;
    setBusy(true);
    try {
      const r = await calibrationApi.clear(p.id);
      toast(`${r.sent} sent to ${p.name} — print the test piece now`);
      onError(null);
    } catch (e) {
      onError(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const save = async (): Promise<SkewRecord | null> => {
    if (!p || !planes) return null;
    try {
      const r = await calibrationApi.saveMeasurement(p.id, planes);
      onError(null);
      return r;
    } catch (e) {
      onError(errText(e));
      return null;
    }
  };

  const apply = async () => {
    if (!p || !preview) return;
    setBusy(true);
    try {
      const rec = await save();
      if (!rec) return;
      const r = await calibrationApi.apply(p.id, rec.id);
      toast(`Skew saved on ${p.name} as profile “foreman” · ${r.restarting ? "Klipper is restarting (SAVE_CONFIG)" : "done"}`);
      setAck(false);
      await onChanged();
    } catch (e) {
      onError(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const saveOnly = async () => {
    setBusy(true);
    const r = await save();
    setBusy(false);
    if (r && p) {
      toast(`Measurement saved for ${p.name} (not sent)`);
      await onChanged();
    }
  };

  return (
    <>
      <section className="tl-sec">
        <h2 className="tl-h2">1 · Printer</h2>
        <label className="tl-field tl-field--narrow">
          <span className="tl-label">Printer</span>
          <select className="tl-input" value={selected ?? ""} onChange={(e) => onSelect(Number(e.target.value))}>
            {printers.map((x) => (
              <option key={x.id} value={x.id}>
                {printerOption(x)}
              </option>
            ))}
          </select>
        </label>
        {p && (
          <div className={`tl-note ${p.supported ? "" : "tl-note--warn"}`}>
            <b>
              {!p.supported
                ? `${p.name}: not supported`
                : p.applied
                  ? `Current: ${skewSummary(p.applied.planes)} · ${shortDate(p.applied.applied_at ?? p.applied.measured_at)} · profile “foreman”`
                  : "No skew correction from Foreman yet"}
            </b>
            <span className="tl-muted">
              {p.supported
                ? `Klipper · ${stateLabel(p)}. Stored on the printer ([skew_correction foreman] in printer.cfg), not on any filament.`
                : p.unsupported}
            </span>
          </div>
        )}
      </section>

      {p?.supported && (
        <>
          <section className="tl-sec">
            <h2 className="tl-h2">2 · Print the skew test</h2>
            <p className="tl-small tl-muted">
              Clear the old correction first so it doesn't hide the error, then print a skew test piece (e.g. the
              Klipper docs' calibration square; tall piece for XZ/YZ) from the Library yourself.
            </p>
            <div className="tl-row">
              <code className="tl-code-inline">SET_SKEW CLEAR=1</code>
              <button type="button" className="tl-btn" disabled={busy || !!sendBlock} onClick={() => void clear()}>
                Send to {p.name}
              </button>
              {sendBlock && <span className="tl-small tl-fg--warn">{sendBlock}</span>}
            </div>
          </section>

          <section className="tl-sec">
            <h2 className="tl-h2">3 · Measure the diagonals</h2>
            <p className="tl-small tl-muted">
              Per plane: corner A→C, B→D, and the side A→D (skip the corner flats). Same three numbers Klipper's SET_SKEW takes.
            </p>
            {(more ? (["XY", "XZ", "YZ"] as const) : (["XY"] as const)).map((pl) => (
              <div key={pl} className="tl-plane">
                <b className="tl-mono tl-plane__name">{pl}</b>
                {(["AC", "BD", "AD"] as const).map((k, j) => (
                  <label key={k} className="tl-field">
                    <span className="tl-label tl-small">{k} mm</span>
                    <input
                      className="tl-input tl-mono"
                      type="text"
                      inputMode="decimal"
                      aria-label={`${pl} ${k} mm`}
                      value={fields[pl][j]}
                      onChange={(e) => {
                        const arr = [...fields[pl]] as [string, string, string];
                        arr[j] = e.target.value;
                        setFields({ ...fields, [pl]: arr });
                      }}
                    />
                  </label>
                ))}
              </div>
            ))}
            <label className="tl-check">
              <input type="checkbox" checked={more} onChange={() => setMore(!more)} /> Also correct XZ and YZ (needs the tall test piece)
            </label>
          </section>

          <section className="tl-sec">
            <h2 className="tl-h2">4 · Result</h2>
            {previewErr && <p className="tl-error">{previewErr}</p>}
            {!preview && !previewErr && <p className="tl-muted tl-small">Enter AC, BD and AD for XY.</p>}
            {preview && (
              <>
                <div className="tl-tiles">
                  {preview.planes.map((r) => (
                    <div key={r.plane} className="tl-tile">
                      <div className="tl-mono tl-small tl-muted">{r.plane} SKEW</div>
                      <div className={`tl-mono tl-big ${Math.abs(r.degrees) > 0.5 ? "tl-fg--warn" : ""}`}>{deg(r.degrees)}</div>
                      <div className="tl-small tl-muted">
                        {Math.abs(r.mm_per_100).toFixed(2)} mm off over 100 mm · factor {r.factor.toFixed(6)}
                      </div>
                    </div>
                  ))}
                </div>
                <div className="tl-small tl-muted">Exactly what Apply sends, in this order:</div>
                <pre className="tl-code" aria-label="Commands">{preview.commands.join("\n")}</pre>
                <div className="tl-note tl-note--warn">
                  <b>SAVE_CONFIG restarts Klipper.</b>
                  <span>
                    It writes [skew_correction foreman] into printer.cfg and restarts the firmware: heaters turn off and the
                    printer loses its homing. Foreman refuses while the printer is printing or paused.
                  </span>
                  <label className="tl-check">
                    <input type="checkbox" checked={ack} onChange={() => setAck(!ack)} /> I understand Klipper will restart
                  </label>
                </div>
                <div className="tl-row">
                  <button
                    type="button"
                    className="tl-btn tl-btn--primary"
                    disabled={busy || !ack || !!sendBlock}
                    onClick={() => void apply()}
                  >
                    Apply to {p.name}
                  </button>
                  <button type="button" className="tl-btn" disabled={busy} onClick={() => void saveOnly()}>
                    Save measurement only
                  </button>
                  {sendBlock && <span className="tl-small tl-fg--warn">{sendBlock}</span>}
                </div>
                <p className="tl-small tl-muted">
                  To check it: print the test again with correction on and enter the new numbers. They should be close to 0°.
                  Keep parts away from the bed edges; corrected moves can reach past them.
                </p>
              </>
            )}
          </section>

          <MacroSection printer={p} toast={toast} onError={onError} />

          <section className="tl-sec">
            <h2 className="tl-h2">Previous results · {p.name}</h2>
            {p.history.length === 0 ? (
              <p className="tl-muted tl-small">None yet.</p>
            ) : (
              <ul className="tl-list">
                {p.history.map((r) => (
                  <li key={r.id} className="tl-list__row">
                    <span className="tl-mono">{skewSummary(r.planes)}</span>
                    <span className="tl-small tl-muted">
                      {r.applied_at ? `applied ${shortDate(r.applied_at)}` : `measured ${shortDate(r.measured_at)} · not sent`} ·{" "}
                      {r.applied_by ?? r.measured_by}
                    </span>
                    <button type="button" className="tl-btn tl-btn--small" onClick={() => remeasure(r)}>
                      Re-measure
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </section>
        </>
      )}
    </>
  );
}

function MacroSection({ printer, toast, onError }: { printer: SkewPrinter; toast: Toast; onError: (e: string | null) => void }) {
  const [patch, setPatch] = useState<MacroPatch | null>(null);
  const [loading, setLoading] = useState(false);
  const [restart, setRestart] = useState(false);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    setPatch(null);
    setRestart(false);
  }, [printer.id]);

  const review = async () => {
    setLoading(true);
    try {
      const p = await calibrationApi.macrosPreview(printer.id);
      setPatch(p);
      setRestart(p.adds_section);
      onError(null);
    } catch (e) {
      onError(errText(e));
    } finally {
      setLoading(false);
    }
  };
  const write = async () => {
    if (!patch) return;
    setBusy(true);
    try {
      const r = await calibrationApi.macrosApply(printer.id, patch, restart);
      toast(`${r.written.length} file${r.written.length === 1 ? "" : "s"} written (.bak kept)${r.restarted ? " · FIRMWARE_RESTART sent" : ""}`);
      setPatch(null);
    } catch (e) {
      onError(errText(e));
    } finally {
      setBusy(false);
    }
  };
  const restartBlocked = printer.state !== "idle";

  return (
    <section className="tl-sec">
      <h2 className="tl-h2">Optional · load the profile in every print</h2>
      <p className="tl-small tl-muted">
        Add <code className="tl-code-inline">SKEW_PROFILE LOAD=foreman</code> to PRINT_START (last line, after homing and purge)
        and <code className="tl-code-inline">SET_SKEW CLEAR=1</code> to PRINT_END, and <code className="tl-code-inline">[skew_correction]</code>{" "}
        to printer.cfg if it's missing. You review the diff first; nothing is written until you press Write.
      </p>
      <div className="tl-row">
        <button type="button" className="tl-btn" disabled={loading} onClick={() => void review()}>
          {loading ? "Reading config…" : patch ? "Re-read config" : "Review changes"}
        </button>
      </div>
      {patch && (
        <>
          <ul className="tl-notes">
            {patch.notes.map((n) => (
              <li key={n}>{n}</li>
            ))}
          </ul>
          {patch.files.length === 0 ? (
            <p className="tl-small tl-fg--ok">✓ Nothing to change.</p>
          ) : (
            <>
              {patch.files.map((f) => (
                <div key={f.path} className="tl-diff">
                  <div className="tl-mono tl-small">{f.path}</div>
                  <pre className="tl-code">
                    {f.diff.map((l, i) => (
                      <span key={i} className={l.startsWith("+ ") ? "tl-diff__add" : l.startsWith("- ") ? "tl-diff__del" : undefined}>
                        {l}
                        {"\n"}
                      </span>
                    ))}
                  </pre>
                </div>
              ))}
              <label className="tl-check">
                <input type="checkbox" checked={restart} disabled={restartBlocked} onChange={() => setRestart(!restart)} /> FIRMWARE_RESTART
                after writing{patch.adds_section ? " (needed: [skew_correction] is new — SET_SKEW won't work until Klipper restarts)" : ""}
              </label>
              {restartBlocked && (
                <span className="tl-small tl-fg--warn">{printer.name} is {printer.state ?? "offline"} — restart not offered.</span>
              )}
              <div className="tl-row">
                <button type="button" className="tl-btn tl-btn--primary" disabled={busy} onClick={() => void write()}>
                  Write {patch.files.length} file{patch.files.length === 1 ? "" : "s"} (.bak first)
                </button>
              </div>
            </>
          )}
        </>
      )}
    </section>
  );
}

// ---------------------------------------------------------------------------
// Shrinkage
// ---------------------------------------------------------------------------

function ShrinkFlow({ printers, filaments, onChanged, toast, onError, goSkew }: {
  printers: SkewPrinter[];
  filaments: ShrinkFilament[];
  onChanged: () => Promise<void>;
  toast: Toast;
  onError: (e: string | null) => void;
  goSkew: (printerId: number) => void;
}) {
  const [fid, setFid] = useState<number | null>(null);
  const [pid, setPid] = useState<number | null>(null);
  const [axes, setAxes] = useState<AxisFields>(EMPTY_AXES);
  const [result, setResult] = useState<ShrinkageResult | null>(null);
  const [resErr, setResErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (fid == null && filaments[0]) setFid(filaments[0].id);
  }, [filaments, fid]);
  useEffect(() => {
    if (pid == null && printers[0]) setPid((printers.find((p) => p.applied) ?? printers[0]).id);
  }, [printers, pid]);

  const f = filaments.find((x) => x.id === fid) ?? null;
  const p = printers.find((x) => x.id === pid) ?? null;
  const input = useMemo(() => shrinkInput(axes), [axes]);

  useEffect(() => {
    if (!input) {
      setResult(null);
      setResErr(null);
      return;
    }
    const t = setTimeout(() => {
      calibrationApi.shrinkPreview(input).then(
        (r) => {
          setResult(r);
          setResErr(null);
        },
        (e) => {
          setResult(null);
          setResErr(errText(e));
        },
      );
    }, 200);
    return () => clearTimeout(t);
  }, [input]);

  const save = async () => {
    if (!f || !input) return;
    setBusy(true);
    try {
      const r = await calibrationApi.saveShrinkage(f.id, input, p?.id ?? null);
      toast(
        `Saved on ${f.name}: shrinkage_xy ${pct(r.result.shrinkage_xy)} · shrinkage_z ${pct(r.result.shrinkage_z)} (orcaslicer_config)`,
      );
      onError(null);
      await onChanged();
    } catch (e) {
      onError(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const skewNote = !p
    ? null
    : !p.supported
      ? { warn: true, text: `Test printer: ${p.name} · no software skew correction. Shrinkage measured here includes any frame skew.`, fix: false }
      : !p.applied
        ? { warn: true, text: `Test printer: ${p.name} · skew not corrected yet. Shrinkage measured here includes the skew error.`, fix: true }
        : { warn: false, text: `Test printer: ${p.name} · skew corrected (${skewSummary(p.applied.planes)}).`, fix: false };

  if (filaments.length === 0) return <p className="tl-muted">No filaments in Spoolman yet.</p>;
  const label = (x: ShrinkFilament) =>
    `${[x.vendor, x.name].filter(Boolean).join(" ")} (#${x.id})${x.shrinkage_xy != null ? ` · XY ${x.shrinkage_xy}%` : ""}`;

  return (
    <>
      <section className="tl-sec tl-grid2">
        <h2 className="tl-h2 tl-span">1 · Filament and printer</h2>
        {skewNote && (
          <div className={`tl-note tl-span ${skewNote.warn ? "tl-note--warn" : ""}`}>
            <span>{skewNote.text}</span>
            {skewNote.fix && p && (
              <button type="button" className="tl-btn tl-btn--small" onClick={() => goSkew(p.id)}>
                Do printer skew first
              </button>
            )}
          </div>
        )}
        <label className="tl-field">
          <span className="tl-label">Filament</span>
          <select className="tl-input" value={fid ?? ""} onChange={(e) => setFid(Number(e.target.value))}>
            {filaments.map((x) => (
              <option key={x.id} value={x.id}>
                {label(x)}
              </option>
            ))}
          </select>
        </label>
        <label className="tl-field">
          <span className="tl-label">Printed on</span>
          <select className="tl-input" value={pid ?? ""} onChange={(e) => setPid(Number(e.target.value))}>
            {printers.map((x) => (
              <option key={x.id} value={x.id}>
                {x.name}
                {x.supported ? (x.applied ? " · skew corrected" : " · skew not corrected") : " · no skew correction"}
              </option>
            ))}
          </select>
        </label>
        {f && (
          <p className="tl-small tl-muted tl-span">
            Current on {f.name}: shrinkage_xy <b className="tl-mono">{pct(f.shrinkage_xy)}</b> · shrinkage_z{" "}
            <b className="tl-mono">{pct(f.shrinkage_z)}</b>
          </p>
        )}
      </section>

      <section className="tl-sec">
        <h2 className="tl-h2">2 · Print the test piece</h2>
        <p className="tl-small tl-muted">
          Print a calibration cube or Calistar star in this filament on the printer above, from the Library yourself.
        </p>
      </section>

      <section className="tl-sec">
        <h2 className="tl-h2">3 · Measure</h2>
        <p className="tl-small tl-muted">Calipers on the outer faces. Leave an axis empty to skip it.</p>
        <div className="tl-axes">
          {(["x", "y", "z"] as Axis[]).map((a) => (
            <div key={a} className="tl-axis">
              <b className="tl-mono">{a.toUpperCase()}</b>
              <label className="tl-field">
                <span className="tl-label tl-small">nominal mm</span>
                <input className="tl-input tl-mono" inputMode="decimal" aria-label={`${a.toUpperCase()} nominal mm`} value={axes[a].nominal}
                  onChange={(e) => setAxes({ ...axes, [a]: { ...axes[a], nominal: e.target.value } })} />
              </label>
              <label className="tl-field">
                <span className="tl-label tl-small">measured mm</span>
                <input className="tl-input tl-mono" inputMode="decimal" aria-label={`${a.toUpperCase()} measured mm`} value={axes[a].measured}
                  onChange={(e) => setAxes({ ...axes, [a]: { ...axes[a], measured: e.target.value } })} />
              </label>
            </div>
          ))}
        </div>
      </section>

      <section className="tl-sec">
        <h2 className="tl-h2">4 · Result</h2>
        {resErr && <p className="tl-error">{resErr}</p>}
        {!result && !resErr && <p className="tl-muted tl-small">Enter at least one measured length.</p>}
        {result && (
          <>
            <div className="tl-tiles">
              {(["x", "y", "z"] as Axis[]).map((a) => {
                const e = result[a];
                const v = result[`${a}_pct` as "x_pct"];
                return (
                  <div key={a} className="tl-tile">
                    <div className="tl-mono tl-small tl-muted">{a.toUpperCase()} CORRECTION</div>
                    <div className="tl-mono tl-big">{v == null ? "—" : `${v.toFixed(2)}%`}</div>
                    <div className="tl-small tl-muted">
                      {e ? `${e.err < 0 ? "shrank" : "grew"} ${Math.abs(e.err * 100).toFixed(2)} %` : "not measured"}
                    </div>
                  </div>
                );
              })}
            </div>
            <p className="tl-small">
              Stored as <span className="tl-mono">shrinkage_xy {pct(result.shrinkage_xy)}</span> (mean of X and Y) ·{" "}
              <span className="tl-mono">shrinkage_z {pct(result.shrinkage_z)}</span> in the filament's orcaslicer_config.
            </p>
            <div className="tl-row">
              <button type="button" className="tl-btn tl-btn--primary" disabled={busy || !f} onClick={() => void save()}>
                Apply to {f?.name ?? "filament"}
              </button>
              <span className="tl-small tl-muted">Every printer uses it; OrcaSlicer export picks it up.</span>
            </div>
          </>
        )}
      </section>

      {f && (
        <section className="tl-sec">
          <h2 className="tl-h2">Previous results · {f.name}</h2>
          {f.history.length === 0 ? (
            <p className="tl-muted tl-small">None yet.</p>
          ) : (
            <ul className="tl-list">
              {f.history.map((r) => (
                <li key={r.id} className="tl-list__row">
                  <span className="tl-mono">
                    XY {pct(r.shrinkage_xy)} · Z {pct(r.shrinkage_z)}
                  </span>
                  <span className="tl-small tl-muted">
                    {shortDate(r.saved_at)} · {r.printer_name ?? "printer not set"}
                    {r.printer_name && !r.printer_skew_corrected ? " (skew not corrected)" : ""}
                  </span>
                  <button type="button" className="tl-btn tl-btn--small" onClick={() => setAxes(axesFrom(r.input))}>
                    Re-measure
                  </button>
                </li>
              ))}
            </ul>
          )}
        </section>
      )}
    </>
  );
}
