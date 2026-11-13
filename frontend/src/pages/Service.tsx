/**
 * Service — maintenance (task 5.3, W12).
 * Reference: docs/ui-ref/Service.dc.html lines 1–113 (layout), 114+ (behaviour).
 *
 * Due work first, then every task by printer. Counters accrue from the job
 * history (never cancelled jobs); Mark done logs the service and resets them.
 */
import { useCallback, useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { useOutletContext, useSearchParams } from "react-router-dom";
import { api, type PrinterWithStatus } from "../api/client";
import {
  amount,
  everyText,
  gapText,
  maintenanceApi,
  MAINTENANCE_TOPICS,
  num,
  shortDate,
  statusText,
  tone,
  type Basis,
  type Overview,
  type ServiceLog,
  type TaskForm,
  type TaskTemplate,
  type TaskView,
} from "../api/maintenance";
import { useEventStream } from "../hooks/useEventStream";
import { usePhone } from "../hooks/useMediaQuery";
import { FilterSheet, PhoneTitle } from "../ui";
import type { ShellContext } from "../shell/Shell";
import { ToastStack, useToasts } from "../ui";
import "./Service.css";

type Filter = "all" | "due" | "over";
type Panel = { kind: "task"; id: number } | { kind: "new"; printerId: number | null } | { kind: "printer"; id: number };

const BASES: [Basis, string][] = [
  ["hours", "Print hours"],
  ["grams", "Grams"],
  ["jobs", "Jobs"],
];

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Data, actions and the inspector, shared by the Service page and the printer page's Service tab. */
function useService() {
  const { openInspector, closeInspector, inspectorOpen, inspectorBodyId } = useOutletContext<ShellContext>();
  const [params, setParams] = useSearchParams();
  const [ov, setOv] = useState<Overview | null>(null);
  const [printers, setPrinters] = useState<PrinterWithStatus[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [panel, setPanel] = useState<Panel | null>(null);
  const { toasts, addToast, removeToast } = useToasts();

  const load = useCallback(async () => {
    try {
      const [o, p] = await Promise.all([maintenanceApi.overview(), api.listPrinters()]);
      setOv(o);
      setPrinters(p);
      setError(null);
    } catch (e) {
      setError(errText(e));
    }
  }, []);
  useEffect(() => void load(), [load]);
  const onEvent = useCallback(() => void load(), [load]);
  useEventStream(onEvent, MAINTENANCE_TOPICS);
  useEffect(() => () => closeInspector(), [closeInspector]);

  const [portal, setPortal] = useState<Element | null>(null);
  useEffect(() => setPortal(document.getElementById(inspectorBodyId)), [inspectorBodyId, inspectorOpen]);

  const show = useCallback(
    (p: Panel) => {
      setPanel(p);
      openInspector();
    },
    [openInspector],
  );
  const close = useCallback(() => {
    setPanel(null);
    closeInspector();
    if (params.has("task")) setParams({}, { replace: true });
  }, [closeInspector, params, setParams]);

  const tasks = ov?.tasks ?? [];
  const printerName = (id: number) => printers.find((p) => p.id === id)?.name ?? `Printer ${id}`;

  const markDone = async (v: TaskView, notes = "") => {
    try {
      const r = await maintenanceApi.done(v.task.id, notes);
      addToast(`${v.task.name} on ${v.printer_name} done · counter reset`, () => {
        void maintenanceApi.undoDone(r.log.id).then(load, (e) => setError(errText(e)));
      });
      await load();
    } catch (e) {
      setError(errText(e));
    }
  };
  const snooze = async (v: TaskView) => {
    const prev = v.task.snooze;
    try {
      await maintenanceApi.snooze(v.task.id);
      addToast(`${v.task.name} snoozed ${amount(v.snooze_amount, v.task.basis)}`, () => {
        void maintenanceApi.snooze(v.task.id, prev).then(load, (e) => setError(errText(e)));
      });
      await load();
    } catch (e) {
      setError(errText(e));
    }
  };

  const selId = panel?.kind === "task" ? panel.id : null;
  const row = (v: TaskView, withPrinter: boolean) => (
    <TaskRow key={`${withPrinter ? "d" : "p"}${v.task.id}`} v={v} withPrinter={withPrinter} selected={selId === v.task.id}
      onOpen={() => show({ kind: "task", id: v.task.id })} onSnooze={() => void snooze(v)} onDone={() => void markDone(v)} />
  );

  const panels = (
    <>
      {portal &&
        inspectorOpen &&
        panel &&
        createPortal(
          (() => {
            const cur = selId != null ? tasks.find((t) => t.task.id === selId) : undefined;
            return panel.kind === "task" && cur ? (
              <TaskInspector key={cur.task.id} v={cur} onClose={close} onDone={(n) => markDone(cur, n)} onChanged={load}
                onDeleted={() => { close(); void load(); }} onError={setError} />
            ) : panel.kind === "new" ? (
              <NewTaskPanel printers={printers} printerId={panel.printerId} onClose={close}
                onCreated={(id) => { void load().then(() => (id != null ? show({ kind: "task", id }) : close())); }} onError={setError} />
            ) : panel.kind === "printer" ? (
              <PrinterHistory printerId={panel.id} name={printerName(panel.id)} onClose={close} />
            ) : null;
          })(),
          portal,
        )}
      <ToastStack toasts={toasts} onDismiss={removeToast} />
    </>
  );

  return { ov, printers, tasks, error, setError, load, panel, show, row, printerName, panels, params };
}

export default function Service() {
  const { ov, printers, tasks, error, setError, load, panel, show, row, printerName, panels, params } = useService();
  const [filter, setFilter] = useState<Filter>("all");

  // Deep link from an Attention item: /service?task=<id>.
  useEffect(() => {
    const id = Number(params.get("task"));
    if (id && ov?.tasks.some((t) => t.task.id === id) && (panel?.kind !== "task" || panel.id !== id)) show({ kind: "task", id });
  }, [params, ov]);

  const due = tasks.filter((t) => t.progress.state !== "ok");
  const over = tasks.filter((t) => t.progress.state === "overdue").length;
  const soon = due.length - over;
  const byPrinter = useMemo(() => {
    const m = new Map<number, TaskView[]>();
    for (const p of printers) m.set(p.id, []);
    for (const t of tasks) m.set(t.task.printer_id, [...(m.get(t.task.printer_id) ?? []), t]);
    for (const list of m.values()) list.sort((a, b) => a.task.name.localeCompare(b.task.name));
    return m;
  }, [tasks, printers]);

  const phone = usePhone();
  const filtered = (list: TaskView[]) =>
    list.filter((t) => filter === "all" || (filter === "due" ? t.progress.state !== "ok" : t.progress.state === "overdue"));
  const dueShown = filtered(due);

  return (
    <div className="svc">
      <div className="svc-head">
        <div className="svc-head__main">
          <h1 className="svc-title phone-dup-title">Service</h1>
          <p className="svc-sub">
            {ov
              ? `${over} overdue · ${soon} due soon · ${tasks.length} task${tasks.length === 1 ? "" : "s"} across ${printers.length} printer${printers.length === 1 ? "" : "s"}`
              : "Loading…"}
          </p>
        </div>
        <button type="button" className="svc-btn" onClick={() => show({ kind: "new", printerId: printers[0]?.id ?? null })}>
          New task
        </button>
      </div>

      <PhoneTitle>
        Service{over + soon > 0 ? <span> · {over + soon} due</span> : null}
      </PhoneTitle>
      <FilterSheet active={filter === "all" ? 0 : 1}>
      <div className="svc-bar">
        <div role="group" aria-label="Show" className="svc-seg">
          {(
            [
              ["all", "All"],
              ["due", "Due & soon"],
              ["over", "Overdue"],
            ] as [Filter, string][]
          ).map(([id, label]) => (
            <button key={id} type="button" aria-pressed={filter === id} onClick={() => setFilter(id)}>
              {label}
            </button>
          ))}
        </div>
        {ov && <DueSoonSetting pct={ov.due_soon_pct} onSaved={load} onError={setError} />}
      </div>
      </FilterSheet>

      {error && (
        <p className="svc-error" role="alert">
          {error}
        </p>
      )}

      <section className="svc-sec" aria-label="Due work">
        <h2 className="svc-sec__title">Due work</h2>
        <div className="svc-list">
          {dueShown.length === 0 ? <p className="svc-empty">✓ Nothing due.</p> : dueShown.map((v) => row(v, true))}
        </div>
      </section>

      <div className="svc-grid auto-cols page-fill">
      {filter === "all" &&
        [...byPrinter.entries()].map(([pid, list]) => (
          <section key={pid} className="svc-sec" aria-label={printerName(pid)}>
            <div className="svc-sec__head">
              <h2 className="svc-sec__title">{printerName(pid)}</h2>
              <button type="button" className="svc-btn svc-btn--small" onClick={() => show({ kind: "printer", id: pid })}>
                History
              </button>
              <button type="button" className="svc-btn svc-btn--small" onClick={() => show({ kind: "new", printerId: pid })}>
                Add task
              </button>
            </div>
            <div className="svc-list">
              {list.length === 0 ? (
                <p className="svc-empty">No service tasks yet — add one or start from the common ones.</p>
              ) : (
                list.map((v) => row(v, false))
              )}
            </div>
          </section>
        ))}
      </div>

      {phone && (
        <button type="button" className="ui-list-add" onClick={() => show({ kind: "new", printerId: printers[0]?.id ?? null })}>
          + New task
        </button>
      )}

      {panels}
    </div>
  );
}


const STATE_ORDER: Record<string, number> = { overdue: 0, soon: 1, ok: 2 };

/** Printer page → Service: this printer's tasks, due first, with Done / Snooze and the same inspector. */
export function PrinterServiceTab({ printerId }: { printerId: number }) {
  const { tasks, error, ov, show, row, panels } = useService();
  const mine = tasks
    .filter((t) => t.task.printer_id === printerId)
    .sort((a, b) => (STATE_ORDER[a.progress.state] ?? 3) - (STATE_ORDER[b.progress.state] ?? 3) || a.task.name.localeCompare(b.task.name));
  const over = mine.filter((t) => t.progress.state === "overdue").length;
  const soon = mine.filter((t) => t.progress.state !== "ok").length - over;
  return (
    <div className="svc svc--printer" role="tabpanel" aria-label="Service">
      <section className="svc-sec page-fill page-fill--col" aria-label="Service tasks">
        <div className="svc-sec__head">
          <h2 className="svc-sec__title">
            {ov ? `${over} overdue · ${soon} due soon · ${mine.length} task${mine.length === 1 ? "" : "s"}` : "Loading…"}
          </h2>
          <button type="button" className="svc-btn svc-btn--small" onClick={() => show({ kind: "printer", id: printerId })}>
            History
          </button>
          <button type="button" className="svc-btn svc-btn--small" onClick={() => show({ kind: "new", printerId })}>
            Add task
          </button>
        </div>
        {error && (
          <p className="svc-error" role="alert">
            {error}
          </p>
        )}
        <div className="svc-list page-fill">
          {ov && mine.length === 0 ? (
            <p className="svc-empty">No service tasks yet — add one or start from the common ones.</p>
          ) : (
            mine.map((v) => row(v, false))
          )}
        </div>
      </section>
      {panels}
    </div>
  );
}

function TaskRow({ v, withPrinter, selected, onOpen, onSnooze, onDone }: {
  v: TaskView; withPrinter: boolean; selected: boolean; onOpen: () => void; onSnooze: () => void; onDone: () => void;
}) {
  const t = tone(v.progress.state);
  const pct = Math.min(100, Math.round(v.progress.ratio * 100));
  return (
    <div className={`svc-row${selected ? " svc-row--sel" : ""}`}>
      <button type="button" className="svc-row__open" onClick={onOpen}>
        <b>{v.task.name}</b>
        <span className="svc-row__meta">
          {withPrinter ? `${v.printer_name} · ` : ""}every {everyText(v.task)}
        </span>
      </button>
      <div className="svc-row__prog">
        <div className="svc-meter" role="progressbar" aria-label={`${v.task.name} progress`} aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100}>
          <div className={`svc-meter__fill svc-tone--${t}`} style={{ width: `${pct}%` }} />
        </div>
        <span className={`svc-row__status svc-fg--${t}`}>{statusText(v)}</span>
      </div>
      <button type="button" className="svc-btn svc-btn--small" onClick={onSnooze} disabled={v.progress.state === "ok"}>
        Snooze {amount(v.snooze_amount, v.task.basis)}
      </button>
      <button type="button" className="svc-btn svc-btn--small svc-btn--strong" onClick={onDone}>
        Done
      </button>
    </div>
  );
}

function DueSoonSetting({ pct, onSaved, onError }: { pct: number; onSaved: () => void; onError: (e: string) => void }) {
  const [v, setV] = useState(String(pct));
  useEffect(() => setV(String(pct)), [pct]);
  const save = async () => {
    const n = Number(v);
    if (!Number.isFinite(n) || n === pct) return;
    try {
      await maintenanceApi.setDueSoon(n);
      onSaved();
    } catch (e) {
      onError(errText(e));
      setV(String(pct));
    }
  };
  return (
    <label className="svc-soon">
      <span>Due soon at</span>
      <input type="number" min={1} max={100} value={v} onChange={(e) => setV(e.target.value)} onBlur={() => void save()}
        onKeyDown={(e) => e.key === "Enter" && void save()} className="svc-input svc-input--num" aria-label="Due soon threshold, percent" />
      <span>%</span>
    </label>
  );
}

function InspHead({ type, title, onClose }: { type: string; title: string; onClose: () => void }) {
  return (
    <div className="svc-insp__head">
      <div className="svc-insp__titles">
        <div className="svc-insp__type">{type}</div>
        <h2 className="svc-insp__title">{title}</h2>
      </div>
      <button type="button" className="svc-insp__close" aria-label="Close" onClick={onClose}>
        ✕
      </button>
    </div>
  );
}

function IntervalFields({ basis, setBasis, interval, setInterval, days, setDays }: {
  basis: Basis; setBasis: (b: Basis) => void; interval: string; setInterval: (s: string) => void; days: string; setDays: (s: string) => void;
}) {
  return (
    <>
      <div className="svc-int">
        <input type="number" min={0} step="any" value={interval} onChange={(e) => setInterval(e.target.value)} aria-label="Interval amount" className="svc-input svc-input--num" />
        <div role="radiogroup" aria-label="Interval unit" className="svc-seg svc-seg--fill">
          {BASES.map(([b, label]) => (
            <button key={b} type="button" role="radio" aria-checked={basis === b} onClick={() => setBasis(b)}>
              {label}
            </button>
          ))}
        </div>
      </div>
      <label className="svc-field svc-field--row">
        <span>…or every</span>
        <input type="number" min={1} step={1} value={days} onChange={(e) => setDays(e.target.value)} placeholder="—" className="svc-input svc-input--num" aria-label="Every N days (optional)" />
        <span>days (optional)</span>
      </label>
    </>
  );
}

function formOf(name: string, notes: string, basis: Basis, interval: string, days: string): TaskForm | string {
  const n = Number(interval);
  if (!name.trim()) return "Name the task.";
  if (!Number.isFinite(n) || n <= 0) return "Interval must be above 0.";
  const d = days.trim() ? Number(days) : null;
  if (d != null && (!Number.isInteger(d) || d <= 0)) return "Days must be a whole number above 0.";
  return { printer_id: null, name: name.trim(), notes: notes.trim() || null, basis, interval: n, every_days: d, start_now: false };
}

function TaskInspector({ v, onClose, onDone, onChanged, onDeleted, onError }: {
  v: TaskView; onClose: () => void; onDone: (notes: string) => Promise<void>; onChanged: () => Promise<void> | void; onDeleted: () => void; onError: (e: string) => void;
}) {
  const t = v.task;
  const [notes, setNotes] = useState("");
  const [busy, setBusy] = useState(false);
  const [name, setName] = useState(t.name);
  const [taskNotes, setTaskNotes] = useState(t.notes ?? "");
  const [basis, setBasis] = useState<Basis>(t.basis);
  const [interval, setInterval] = useState(String(t.interval));
  const [days, setDays] = useState(t.every_days != null ? String(t.every_days) : "");
  const [hist, setHist] = useState<ServiceLog[] | null>(null);
  const [formErr, setFormErr] = useState<string | null>(null);
  const loadHist = useCallback(() => maintenanceApi.history({ taskId: t.id }).then(setHist, (e) => onError(errText(e))), [t.id, onError]);
  useEffect(() => void loadHist(), [loadHist, v.last_done]);
  const dirty = name !== t.name || taskNotes !== (t.notes ?? "") || basis !== t.basis || Number(interval) !== t.interval || days !== (t.every_days != null ? String(t.every_days) : "");
  const save = async () => {
    const f = formOf(name, taskNotes, basis, interval, days);
    if (typeof f === "string") return setFormErr(f);
    setFormErr(null);
    try {
      await maintenanceApi.update(t.id, f);
      await onChanged();
    } catch (e) {
      setFormErr(errText(e));
    }
  };
  const k = tone(v.progress.state);
  return (
    <div className="svc-insp">
      <InspHead type={`Service task · ${v.printer_name}`} title={t.name} onClose={onClose} />
      <span className={`svc-chip svc-chip--${k}`}>{statusText(v)}</span>

      <section className="svc-box">
        <h3>Mark done</h3>
        <label className="svc-field">
          <span className="svc-field__label">Note (optional)</span>
          <textarea className="svc-input svc-textarea" placeholder="What did you do? Parts replaced?" value={notes} onChange={(e) => setNotes(e.target.value)} />
        </label>
        <button type="button" className="svc-btn svc-btn--primary" disabled={busy}
          onClick={async () => { setBusy(true); await onDone(notes); setNotes(""); setBusy(false); }}>
          Done · reset counter
        </button>
      </section>

      <section className="svc-box">
        <h3>Since last service</h3>
        <div className="svc-fact"><span>Print hours</span><span>{num(v.counters.hours)} h</span></div>
        <div className="svc-fact"><span>Filament</span><span>{amount(v.counters.grams, "grams")}</span></div>
        <div className="svc-fact"><span>Jobs completed</span><span>{v.counters.jobs}</span></div>
        <div className="svc-fact"><span>Days</span><span>{num(Math.floor(v.progress.days))}</span></div>
        <div className="svc-fact"><span>Counting from</span><span>{t.since ? shortDate(t.since) : "all job history"}</span></div>
      </section>

      <section className="svc-box">
        <h3>Interval</h3>
        <IntervalFields basis={basis} setBasis={setBasis} interval={interval} setInterval={setInterval} days={days} setDays={setDays} />
        <label className="svc-field">
          <span className="svc-field__label">Name</span>
          <input className="svc-input" value={name} onChange={(e) => setName(e.target.value)} />
        </label>
        <label className="svc-field">
          <span className="svc-field__label">Notes</span>
          <textarea className="svc-input svc-textarea" value={taskNotes} onChange={(e) => setTaskNotes(e.target.value)} />
        </label>
        {formErr && <p className="svc-error" role="alert">{formErr}</p>}
        <button type="button" className="svc-btn svc-btn--small" disabled={!dirty} onClick={() => void save()}>
          Save changes
        </button>
      </section>

      <section className="svc-box">
        <h3>History</h3>
        {hist == null ? (
          <p className="svc-empty">Loading…</p>
        ) : (
          <>
            {hist.map((h) => (
              <div key={h.id} className="svc-hist">
                <span className="svc-hist__d">{shortDate(h.done_at)}</span>
                <span className="svc-hist__x">
                  <span>{h.notes ?? "Done"} <span className="svc-hist__by">· {h.done_by}</span></span>
                  <span className="svc-hist__gap">{gapText(h)}</span>
                </span>
              </div>
            ))}
            <div className="svc-hist">
              <span className="svc-hist__d">{shortDate(t.created_at)}</span>
              <span className="svc-hist__x">Task added</span>
            </div>
          </>
        )}
      </section>

      <button type="button" className="svc-btn svc-btn--small svc-btn--danger"
        onClick={async () => {
          if (!window.confirm(`Delete "${t.name}" on ${v.printer_name}? Its service history is kept.`)) return;
          try { await maintenanceApi.remove(t.id); onDeleted(); } catch (e) { onError(errText(e)); }
        }}>
        Delete task
      </button>
    </div>
  );
}

function NewTaskPanel({ printers, printerId, onClose, onCreated, onError }: {
  printers: PrinterWithStatus[]; printerId: number | null; onClose: () => void; onCreated: (id: number | null) => void; onError: (e: string) => void;
}) {
  const [pid, setPid] = useState<number | null>(printerId);
  useEffect(() => setPid(printerId), [printerId]);
  const [name, setName] = useState("");
  const [notes, setNotes] = useState("");
  const [basis, setBasis] = useState<Basis>("hours");
  const [interval, setInterval] = useState("200");
  const [days, setDays] = useState("");
  const [startNow, setStartNow] = useState(false);
  const [templates, setTemplates] = useState<TaskTemplate[]>([]);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [formErr, setFormErr] = useState<string | null>(null);
  useEffect(() => void maintenanceApi.templates().then(setTemplates, (e) => onError(errText(e))), [onError]);

  const create = async () => {
    if (pid == null) return setFormErr("Pick a printer.");
    const f = formOf(name, notes, basis, interval, days);
    if (typeof f === "string") return setFormErr(f);
    try {
      const v = await maintenanceApi.create({ ...f, printer_id: pid, start_now: startNow });
      onCreated(v.task.id);
    } catch (e) {
      setFormErr(errText(e));
    }
  };
  const addTemplates = async () => {
    if (pid == null) return setFormErr("Pick a printer.");
    try {
      await maintenanceApi.fromTemplates(pid, [...picked], startNow);
      onCreated(null);
    } catch (e) {
      setFormErr(errText(e));
    }
  };
  return (
    <div className="svc-insp">
      <InspHead type="Service task" title="New task" onClose={onClose} />
      <label className="svc-field">
        <span className="svc-field__label">Printer</span>
        <select className="svc-input" value={pid ?? ""} onChange={(e) => setPid(e.target.value ? Number(e.target.value) : null)}>
          {printers.map((p) => (
            <option key={p.id} value={p.id}>{p.name}</option>
          ))}
        </select>
      </label>
      <label className="svc-check">
        <input type="checkbox" checked={startNow} onChange={(e) => setStartNow(e.target.checked)} />
        <span>Just serviced — start counting from now (otherwise the printer's whole job history counts)</span>
      </label>

      <section className="svc-box">
        <h3>Common tasks</h3>
        {templates.map((t) => (
          <label key={t.key} className="svc-check">
            <input type="checkbox" checked={picked.has(t.key)}
              onChange={(e) => setPicked((s) => { const n = new Set(s); if (e.target.checked) n.add(t.key); else n.delete(t.key); return n; })} />
            <span><b>{t.name}</b> <span className="svc-row__meta">every {everyText(t)}</span></span>
          </label>
        ))}
        <button type="button" className="svc-btn svc-btn--small" disabled={picked.size === 0} onClick={() => void addTemplates()}>
          Add {picked.size || ""} selected
        </button>
      </section>

      <section className="svc-box">
        <h3>Custom task</h3>
        <label className="svc-field">
          <span className="svc-field__label">Name</span>
          <input className="svc-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. Belt tension" />
        </label>
        <IntervalFields basis={basis} setBasis={setBasis} interval={interval} setInterval={setInterval} days={days} setDays={setDays} />
        <label className="svc-field">
          <span className="svc-field__label">Notes (optional)</span>
          <textarea className="svc-input svc-textarea" value={notes} onChange={(e) => setNotes(e.target.value)} />
        </label>
        <button type="button" className="svc-btn svc-btn--primary" onClick={() => void create()}>
          Add task
        </button>
      </section>
      {formErr && <p className="svc-error" role="alert">{formErr}</p>}
    </div>
  );
}

function PrinterHistory({ printerId, name, onClose }: { printerId: number; name: string; onClose: () => void }) {
  const [hist, setHist] = useState<ServiceLog[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => void maintenanceApi.history({ printerId }).then(setHist, (e) => setErr(errText(e))), [printerId]);
  return (
    <div className="svc-insp">
      <InspHead type={`Service history · ${name}`} title={name} onClose={onClose} />
      {err && <p className="svc-error" role="alert">{err}</p>}
      <section className="svc-box">
        <h3>All services, newest first</h3>
        {hist == null ? (
          <p className="svc-empty">Loading…</p>
        ) : hist.length === 0 ? (
          <p className="svc-empty">No services logged yet.</p>
        ) : (
          hist.map((h) => (
            <div key={h.id} className="svc-hist">
              <span className="svc-hist__d">{shortDate(h.done_at)}</span>
              <span className="svc-hist__x">
                <span><b>{h.task_name}</b> · {h.notes ?? "Done"} <span className="svc-hist__by">· {h.done_by}</span></span>
                <span className="svc-hist__gap">{gapText(h)}</span>
              </span>
            </div>
          ))
        )}
      </section>
    </div>
  );
}
