/** Jobs → History and Stats tabs (docs/ui-ref/Jobs.dc.html lines 105–209, task 3.6). */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { api, type PrinterWithStatus } from "../api/client";
import { FAIL_REASONS, jobs, type Job, type JobOutcome, type JobStats } from "../api/jobs";
import { useEventStream } from "../hooks/useEventStream";
import { useSignedUrl } from "../hooks/useSignedUrl";
import { FilterSheet, HeaderActions, HeaderSearch, StatusChip, relTime, type Tone } from "../ui";
import { usePhone } from "../hooks/useMediaQuery";

const JOB_TOPICS = ["job"];

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export function fmtDuration(secs: number | null | undefined): string {
  if (secs == null) return "—";
  const m = Math.round(secs / 60);
  if (m < 60) return `${m} m`;
  return `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, "0")} m`;
}

export function fmtGrams(g: number | null | undefined): string {
  if (g == null) return "—";
  if (g >= 1000) return `${(g / 1000).toFixed(2)} kg`;
  return `${g < 10 ? g.toFixed(1) : Math.round(g)} g`;
}

function fmtWhen(iso: string | null): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const now = new Date();
  const day = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((day(now) - day(d)) / 86_400_000);
  const hm = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (diff === 0) return `Today ${hm}`;
  if (diff === 1) return `Yesterday ${hm}`;
  if (diff < 7) return `${d.toLocaleDateString([], { weekday: "short" })} ${hm}`;
  return d.toLocaleDateString([], { year: "numeric", month: "short", day: "numeric" });
}

/** Outcome chip: tone, symbol, word. */
export function outcomeChip(j: Job): { tone: Tone; sym: string; word: string } {
  if (j.end === "running") return { tone: "info", sym: "▶", word: "Printing" };
  switch (j.outcome) {
    case "good":
      return { tone: "ok", sym: "✓", word: "Good" };
    case "failed":
      return { tone: "bad", sym: "✖", word: j.fail_reason ? `Failed · ${j.fail_reason}` : "Failed" };
    case "cancelled":
      return { tone: "off", sym: "○", word: "Cancelled" };
    default:
      return { tone: "warn", sym: "◌", word: "Not rated" };
  }
}

function spoolText(j: Job): string {
  const ids = j.tools.map((t) => t.spool_id).filter((x): x is number => x != null);
  return ids.length ? ids.map((i) => `#${i}`).join(" · ") : "";
}

function Thumb({ path }: { path: string | null }) {
  const url = useSignedUrl(path);
  const [broken, setBroken] = useState(false);
  if (!url || broken) return <span className="jh-thumb jh-thumb--blank" aria-hidden="true" />;
  return <img className="jh-thumb" src={url} alt="" width={48} height={36} onError={() => setBroken(true)} />;
}

/** Refetch (debounced) on job events. */
function useJobEvents(reload: () => void) {
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const onEvent = useCallback(
    (ev: { type: string }) => {
      if (ev.type !== "job") return;
      if (timer.current) clearTimeout(timer.current);
      timer.current = setTimeout(reload, 200);
    },
    [reload],
  );
  useEventStream(onEvent, JOB_TOPICS);
  useEffect(() => () => void (timer.current && clearTimeout(timer.current)), []);
}

function usePrinters(): PrinterWithStatus[] {
  const [printers, setPrinters] = useState<PrinterWithStatus[]>([]);
  useEffect(() => {
    api.listPrinters().then(setPrinters).catch(() => {});
  }, []);
  return printers;
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

const OUTCOMES: [string, string][] = [
  ["all", "All"],
  ["good", "Good"],
  ["failed", "Failed"],
  ["cancelled", "Cancelled"],
  ["unrated", "Not rated"],
];

export function JobsHistory({ notify }: { notify: (text: string) => void }) {
  const phone = usePhone();
  const printers = usePrinters();
  const [q, setQ] = useState("");
  const [outcome, setOutcome] = useState("all");
  const [printerId, setPrinterId] = useState<number | null>(null);
  const [material, setMaterial] = useState("");
  const [spool, setSpool] = useState("");
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [list, setList] = useState<Job[]>([]);
  const [total, setTotal] = useState(0);
  const [err, setErr] = useState<string | null>(null);
  const [sel, setSel] = useState<number | null>(null);

  const spoolId = /^\d+$/.test(spool.trim().replace(/^#/, "")) ? Number(spool.trim().replace(/^#/, "")) : null;

  const load = useCallback(async () => {
    try {
      const r = await jobs.list({
        q: q.trim() || undefined,
        outcome,
        printer_id: printerId,
        material: material.trim() || undefined,
        spool_id: spoolId,
        from: from || undefined,
        to: to || undefined,
        limit: 500,
      });
      setList(r.jobs);
      setTotal(r.total);
      setErr(null);
    } catch (e) {
      setErr(errText(e));
    }
  }, [q, outcome, printerId, material, spoolId, from, to]);

  useEffect(() => {
    const t = setTimeout(() => void load(), 150);
    return () => clearTimeout(t);
  }, [load]);
  useJobEvents(load);

  const materials = useMemo(() => {
    const s = new Set<string>();
    for (const j of list) for (const t of j.tools) if (t.material) s.add(t.material.toUpperCase());
    return [...s].sort();
  }, [list]);

  const selected = list.find((j) => j.id === sel) ?? null;

  const rate = async (j: Job, o: JobOutcome, reason?: string | null) => {
    try {
      const upd = await jobs.rate(j.id, o, reason);
      setList((prev) => prev.map((x) => (x.id === upd.id ? upd : x)));
      notify(`${j.file_name} marked ${outcomeChip(upd).word}`);
    } catch (e) {
      notify(`Could not rate: ${errText(e)}`);
    }
  };

  const remove = async (j: Job) => {
    if (!window.confirm(`Delete the history record for ${j.file_name}? Spool weights are not changed.`)) return;
    try {
      await jobs.remove(j.id);
      setSel(null);
      notify(`Deleted record ${j.file_name}`);
      void load();
    } catch (e) {
      notify(`Could not delete: ${errText(e)}`);
    }
  };

  const filtered = q || outcome !== "all" || printerId != null || material || spoolId != null || from || to;
  const nFilters = [outcome !== "all", printerId != null, !!material, spoolId != null, !!from, !!to].filter(Boolean).length;

  return (
    <div className="jh-layout">
      <main className="jobs-main jh-main">
        <HeaderActions>
          <HeaderSearch value={q} onChange={setQ} label="Search history" />
        </HeaderActions>
        <FilterSheet active={nFilters}>
        <div className="jh-toolbar">
          <label className="jh-search">
            <span className="jh-sr">Search history</span>
            <input type="search" placeholder="Search file…" value={q} onChange={(e) => setQ(e.target.value)} />
          </label>
          <div role="group" aria-label="Outcome" className="jh-seg">
            {OUTCOMES.map(([id, label]) => (
              <button
                key={id}
                type="button"
                aria-pressed={outcome === id}
                className={`jh-seg__btn${outcome === id ? " jh-seg__btn--on" : ""}`}
                onClick={() => setOutcome(id)}
              >
                {label}
              </button>
            ))}
          </div>
        </div>
        <div className="jh-toolbar">
          <select
            aria-label="Printer"
            className="jh-select"
            value={printerId ?? ""}
            onChange={(e) => setPrinterId(e.target.value ? Number(e.target.value) : null)}
          >
            <option value="">All printers</option>
            {printers.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
          <label className="jh-field">
            <span className="jh-field__k">Material</span>
            <input
              list="jh-materials"
              value={material}
              placeholder="Any"
              onChange={(e) => setMaterial(e.target.value)}
              size={8}
            />
            <datalist id="jh-materials">
              {materials.map((m) => (
                <option key={m} value={m} />
              ))}
            </datalist>
          </label>
          <label className="jh-field">
            <span className="jh-field__k">Spool</span>
            <input value={spool} placeholder="#id" onChange={(e) => setSpool(e.target.value)} size={5} />
          </label>
          <label className="jh-field">
            <span className="jh-field__k">From</span>
            <input type="date" value={from} onChange={(e) => setFrom(e.target.value)} />
          </label>
          <label className="jh-field">
            <span className="jh-field__k">To</span>
            <input type="date" value={to} onChange={(e) => setTo(e.target.value)} />
          </label>
          {filtered && (
            <button
              type="button"
              className="jh-clear"
              onClick={() => {
                setQ("");
                setOutcome("all");
                setPrinterId(null);
                setMaterial("");
                setSpool("");
                setFrom("");
                setTo("");
              }}
            >
              Clear filters
            </button>
          )}
        </div>
        </FilterSheet>
        <div className="jh-count">
          {total} {total === 1 ? "job" : "jobs"}
        </div>
        {err && <p className="jobs-err">{err}</p>}
        <div className="jh-table-wrap">
          <table className="jh-table ph-table ph-hist">
            <thead>
              <tr>
                <th>File</th>
                <th>Printer</th>
                <th>Finished</th>
                <th className="jh-num ph-hide">Time</th>
                <th className="jh-num ph-hide">Filament</th>
                <th>Outcome</th>
              </tr>
            </thead>
            <tbody>
              {list.length === 0 && (
                <tr>
                  <td colSpan={6} className="jh-empty">
                    {filtered ? "No jobs match these filters." : "No jobs yet. Prints show up here when they start."}
                  </td>
                </tr>
              )}
              {list.map((j) => {
                const c = outcomeChip(j);
                const sp = spoolText(j);
                return (
                  <tr key={j.id} className={sel === j.id ? "jh-row--sel" : undefined}>
                    <td>
                      <span className="jh-file">
                        <Thumb path={j.thumb_url} />
                        <button type="button" className="jh-file__btn" onClick={() => setSel(j.id)}>
                          {j.file_name}
                        </button>
                      </span>
                    </td>
                    <td>{j.printer_name}</td>
                    <td className="jh-muted jh-nowrap">{j.end === "running" ? "printing…" : phone ? relTime(j.ended_at) : fmtWhen(j.ended_at)}</td>
                    <td className="jh-num jh-mono ph-hide">{fmtDuration(j.duration_secs)}</td>
                    <td className="jh-num jh-mono ph-hide">
                      {fmtGrams(j.grams_used ?? j.grams_planned)}
                      {sp && <span className="jh-sub">{sp}</span>}
                    </td>
                    <td className="jh-nowrap">
                      <StatusChip tone={c.tone} symbol={c.sym} word={c.word} short={phone ? "" : undefined} />
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      </main>
      {selected && (
        <JobInspector
          job={selected}
          onClose={() => setSel(null)}
          onRate={(o, r) => void rate(selected, o, r)}
          onDelete={() => void remove(selected)}
        />
      )}
    </div>
  );
}

function JobInspector({
  job: j,
  onClose,
  onRate,
  onDelete,
}: {
  job: Job;
  onClose: () => void;
  onRate: (o: JobOutcome, reason?: string | null) => void;
  onDelete: () => void;
}) {
  const facts: [string, string][] = [
    ["Printer", j.printer_name],
    ["Started", fmtWhen(j.started_at)],
    ["Finished", j.end === "running" ? "printing…" : fmtWhen(j.ended_at)],
    ["Duration", fmtDuration(j.duration_secs)],
    ["Ended as", j.end],
  ];
  for (const t of j.tools) {
    const label = j.tools.length > 1 ? `Tool ${t.index}` : "Filament";
    const used = fmtGrams(t.grams_used ?? t.grams_planned);
    facts.push([
      label,
      [t.material, used, t.spool_id != null ? `spool #${t.spool_id}` : "no spool"].filter(Boolean).join(" · "),
    ]);
  }
  facts.push([
    "Spool deduction",
    j.usage_reporter === "moonraker"
      ? "reported by Moonraker"
      : j.grams_deducted > 0
        ? `${fmtGrams(j.grams_deducted)} by Foreman`
        : j.end === "running"
          ? "when it ends"
          : "none (no spool linked)",
  ]);
  if (j.error) facts.push(["Error", j.error]);

  const cur = j.outcome;
  const outs: [JobOutcome, string][] = [
    ["good", "✓ Good"],
    ["failed", "✖ Failed"],
    ["cancelled", "○ Cancelled"],
  ];
  return (
    <aside aria-label="Job details" className="jh-insp">
      <div className="jh-insp__head">
        <div className="jh-insp__title">
          <div className="jh-cap">{j.end === "running" ? "Running job" : "Finished job"}</div>
          <h2>{j.file_name}</h2>
        </div>
        <button type="button" className="jh-insp__close" aria-label="Close details" onClick={onClose}>
          ✕
        </button>
      </div>
      <div className="jh-facts">
        {facts.map(([k, v]) => (
          <div key={k} className="jh-fact">
            <span className="jh-muted">{k}</span>
            <span className="jh-fact__v">{v}</span>
          </div>
        ))}
      </div>
      {j.end !== "running" && (
        <>
          <div className="jh-cap">Outcome</div>
          <div role="group" aria-label="Outcome" className="jh-outs">
            {outs.map(([o, label]) => (
              <button
                key={o}
                type="button"
                aria-pressed={cur === o}
                className={`jh-seg__btn jh-outs__btn${cur === o ? " jh-seg__btn--on" : ""}`}
                onClick={() => onRate(o, o === "failed" ? j.fail_reason : null)}
              >
                {label}
              </button>
            ))}
          </div>
          {cur === "failed" && (
            <div role="group" aria-label="Failure reason" className="jh-reasons">
              {FAIL_REASONS.map((r) => (
                <button
                  key={r}
                  type="button"
                  aria-pressed={j.fail_reason === r}
                  className={`jh-seg__btn jh-reason${j.fail_reason === r ? " jh-seg__btn--on" : ""}`}
                  onClick={() => onRate("failed", r)}
                >
                  {r}
                </button>
              ))}
            </div>
          )}
        </>
      )}
      <div className="jh-insp__spacer" />
      <div className="jh-insp__foot">
        {j.file_id != null && (
          <Link className="jobs-add jh-primary" to="/library">
            Reprint → Library
          </Link>
        )}
        {j.end !== "running" && (
          <button type="button" className="jh-danger" onClick={onDelete}>
            Delete record
          </button>
        )}
      </div>
    </aside>
  );
}

// ---------------------------------------------------------------------------
// Stats
// ---------------------------------------------------------------------------

const PERIODS: [number, string][] = [
  [7, "7 days"],
  [30, "30 days"],
  [90, "90 days"],
  [365, "1 year"],
];

function pct(v: number | null | undefined): string {
  return v == null ? "—" : `${Math.round(v * 100)}%`;
}

export function JobsStats() {
  const [days, setDays] = useState(7);
  const [stats, setStats] = useState<JobStats | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const printers = usePrinters();

  const load = useCallback(async () => {
    try {
      const from = new Date(Date.now() - days * 86_400_000).toISOString();
      setStats(await jobs.stats(from, new Date().toISOString()));
      setErr(null);
    } catch (e) {
      setErr(errText(e));
    }
  }, [days]);
  useEffect(() => void load(), [load]);
  useJobEvents(load);

  const s = stats;
  const matSub = s && s.grams > 0
    ? s.by_material
        .slice(0, 2)
        .map((m) => `${m.key} ${Math.round((m.grams / s.grams) * 100)}%`)
        .join(" · ")
    : "no filament recorded";
  const spoolsUsed = s ? s.by_spool.filter((x) => x.spool_id != null) : [];
  const kpis = s
    ? [
        { label: `Printed · ${days} d`, value: `${s.hours < 10 ? s.hours.toFixed(1) : Math.round(s.hours)} h`, sub: `${s.jobs} ${s.jobs === 1 ? "job" : "jobs"}` },
        { label: "Success rate", value: pct(s.success_rate), sub: `${s.failed} failed · ${s.cancelled} cancelled · ${s.unrated} not rated` },
        { label: "Filament used", value: fmtGrams(s.grams), sub: matSub },
        { label: "Spools used", value: String(spoolsUsed.length), sub: spoolsUsed[0] ? `most: #${spoolsUsed[0].spool_id} ${fmtGrams(spoolsUsed[0].grams)}` : "none linked" },
      ]
    : [];

  // Every printer gets a utilization row, even with no jobs in the period.
  const rows = useMemo(() => {
    if (!s) return [];
    const seen = new Set<number>();
    const out = s.printers.map((p) => {
      if (p.printer_id != null) seen.add(p.printer_id);
      return p;
    });
    for (const p of printers) {
      if (!seen.has(p.id)) {
        out.push({ printer_id: p.id, name: p.name, hours: 0, jobs: 0, good: 0, failed: 0, cancelled: 0, failure_rate: 0, utilization_pct: 0 });
      }
    }
    return out;
  }, [s, printers]);

  const maxMat = s ? Math.max(1, ...s.by_material.map((m) => m.grams)) : 1;

  return (
    <main className="jobs-main">
      <div className="jh-toolbar">
        <div role="group" aria-label="Period" className="jh-seg">
          {PERIODS.map(([d, label]) => (
            <button
              key={d}
              type="button"
              aria-pressed={days === d}
              className={`jh-seg__btn${days === d ? " jh-seg__btn--on" : ""}`}
              onClick={() => setDays(d)}
            >
              {label}
            </button>
          ))}
        </div>
      </div>
      {err && <p className="jobs-err">{err}</p>}
      <div className="jh-kpis">
        {kpis.map((k) => (
          <div key={k.label} className="jh-kpi">
            <div className="jh-cap">{k.label}</div>
            <div className="jh-kpi__v">{k.value}</div>
            <div className="jh-kpi__sub">{k.sub}</div>
          </div>
        ))}
      </div>
      <section className="jh-panel" aria-labelledby="jh-util">
        <h2 id="jh-util" className="jh-cap">
          Utilization · last {days} days
        </h2>
        {rows.length === 0 && <p className="jh-muted">No printers.</p>}
        {rows.map((p) => (
          <div key={p.printer_id ?? p.name} className="jh-bar">
            <span className="jh-bar__name">{p.name}</span>
            <div className="jh-bar__track" aria-hidden="true">
              <div className="jh-bar__fill" style={{ width: `${Math.min(100, p.utilization_pct)}%` }} />
            </div>
            <span className="jh-bar__val">
              {Math.round(p.utilization_pct)}% · {p.hours.toFixed(1)} h · {p.jobs} {p.jobs === 1 ? "job" : "jobs"} ·{" "}
              {p.failed} {p.failed === 1 ? "fail" : "fails"}
              {p.jobs - p.cancelled > 0 && ` (${pct(p.failure_rate)})`}
            </span>
          </div>
        ))}
      </section>
      <div className="jh-two">
        <section className="jh-panel" aria-labelledby="jh-mat">
          <h2 id="jh-mat" className="jh-cap">
            Filament by material
          </h2>
          {s && s.by_material.length === 0 && <p className="jh-muted">Nothing printed in this period.</p>}
          {s?.by_material.map((m) => (
            <div key={m.key} className="jh-bar">
              <span className="jh-bar__name">{m.key}</span>
              <div className="jh-bar__track" aria-hidden="true">
                <div className="jh-bar__fill" style={{ width: `${(m.grams / maxMat) * 100}%` }} />
              </div>
              <span className="jh-bar__val">
                {fmtGrams(m.grams)} · {m.jobs} {m.jobs === 1 ? "job" : "jobs"}
              </span>
            </div>
          ))}
        </section>
        <section className="jh-panel" aria-labelledby="jh-spool">
          <h2 id="jh-spool" className="jh-cap">
            Filament by spool
          </h2>
          {s && s.by_spool.length === 0 && <p className="jh-muted">Nothing printed in this period.</p>}
          {s?.by_spool.map((u) => (
            <div key={`${u.spool_id ?? "none"}-${u.key}`} className="jh-fact">
              <span>{u.spool_id != null ? `#${u.spool_id} · ${u.key}` : `No spool · ${u.key}`}</span>
              <span className="jh-mono">
                {fmtGrams(u.grams)} · {u.jobs} {u.jobs === 1 ? "job" : "jobs"}
              </span>
            </div>
          ))}
        </section>
      </div>
    </main>
  );
}
