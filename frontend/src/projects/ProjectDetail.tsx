/**
 * Project detail (task 5.1). Reference: docs/ui-ref/Projects.dc.html lines 72–244 +
 * renderVals. Plates with slots and copies, live plate status, progress, time and
 * grams, materials with swap, activity, "print all remaining" → fleet queue.
 */
import { useCallback, useEffect, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import {
  fitText,
  hours,
  isManualSwaps,
  kindLabel,
  plateStatus,
  progressText,
  projectsApi,
  type ProjectPlan,
  type ProjectView,
} from "../api/projects";
import { useEventStream } from "../hooks/useEventStream";
import { dueLabel } from "./ProjectList";
import Thumb, { Dot } from "./Thumb";
import { money } from "../api/inventory";

const TOPICS = ["queue", "job", "spool"];

function when(at: string): string {
  const d = new Date(at);
  const today = new Date().toDateString() === d.toDateString();
  return today ? d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" }) : d.toLocaleDateString(undefined, { day: "numeric", month: "short" });
}

export default function ProjectDetail() {
  const { id } = useParams();
  const pid = Number(id);
  const nav = useNavigate();
  const [p, setP] = useState<ProjectView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [editName, setEditName] = useState<string | null>(null);
  const [swap, setSwap] = useState<string | null>(null);
  const [plan, setPlan] = useState<ProjectPlan | null>(null);
  const [swapAsk, setSwapAsk] = useState<string | null>(null);

  const load = useCallback(() => {
    projectsApi.get(pid).then(
      (v) => {
        setP(v);
        setError(null);
      },
      (e) => setError(e instanceof Error ? e.message : String(e)),
    );
  }, [pid]);
  useEffect(load, [load]);
  useEventStream(load, TOPICS);
  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), 5000);
    return () => clearTimeout(t);
  }, [toast]);
  // Spool options for Swap come from the template's plan (free grams, fit hints).
  useEffect(() => {
    if (!swap || !p?.template_id) return;
    projectsApi.plan(p.template_id, p.sets).then(setPlan, () => setPlan(null));
  }, [swap, p?.template_id, p?.sets]);

  const run = async (f: () => Promise<ProjectView | void>, msg?: string) => {
    setBusy(true);
    setError(null);
    try {
      const v = await f();
      if (v) setP(v);
      else load();
      if (msg) setToast(msg);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  if (!p) return <div className="prj">{error ? <p className="prj-note prj-note--bad">{error}</p> : <p className="prj-muted">Loading…</p>}</div>;
  const t = p.totals;
  // Finished and archived projects are read-only records; Reopen to change them.
  const ro = p.status !== "active";
  const pct = (n: number) => (t.prints ? (n / t.prints) * 100 : 0);
  const due = dueLabel(p);
  const hex = (key: string) => {
    const s = p.slots.find((x) => x.key === key);
    return s?.spool?.color_hex ?? s?.spool?.multi_color_hexes[0] ?? s?.hex ?? null;
  };
  const planned = p.plates.reduce((a, pl) => a + (pl.file.print_time_secs ?? 0) * pl.counts.copies, 0);

  return (
    <div className="prj">
      <Link className="prj-back" to="/projects">
        ← Projects
      </Link>
      <div className="prj-head">
        <div className="prj-head__main">
          {editName != null ? (
            <form
              style={{ display: "flex", gap: 8 }}
              onSubmit={(e) => {
                e.preventDefault();
                void run(() => projectsApi.update(pid, { name: editName, due_date: null, status: null })).then(() => setEditName(null));
              }}
            >
              <input className="prj-input" aria-label="Project name" value={editName} autoFocus onChange={(e) => setEditName(e.target.value)} />
              <button type="submit" className="prj-btn">
                Done
              </button>
            </form>
          ) : (
            <h1 className="prj-title">
              {p.name}{" "}
              {!ro && (
                <button type="button" className="prj-btn prj-btn--small" aria-label="Rename project" onClick={() => setEditName(p.name)}>
                  ✎
                </button>
              )}
            </h1>
          )}
          <div className="pd-facts">
            {p.template_id != null && (
              <span>
                From template <Link to={`/projects/templates/${p.template_id}`}>{p.template_name}</Link>
                {p.sets > 1 ? ` × ${p.sets} sets` : ""}
              </span>
            )}
            <label>
              Due{" "}
              <input
                type="date"
                className="prj-input"
                style={{ width: "auto" }}
                disabled={ro}
                value={p.due_date ?? ""}
                onChange={(e) => void run(() => projectsApi.update(pid, { name: null, due_date: e.target.value, status: null }))}
              />
            </label>
            <span className={`prj-chip prj-tone--${due.tone}`}>{due.text}</span>
          </div>
        </div>
        {!ro && (
          <button
            type="button"
            className="prj-btn prj-btn--primary"
            disabled={busy || t.remaining === 0}
            onClick={() =>
              void run(async () => {
                try {
                  const r = await projectsApi.printRemaining(pid);
                  setToast(`${r.added} print${r.added === 1 ? "" : "s"} added to the fleet queue`);
                } catch (e) {
                  if (isManualSwaps(e)) setSwapAsk(e.message);
                  else throw e;
                }
              })
            }
          >
            {t.remaining ? `Print all remaining · ${t.remaining} print${t.remaining === 1 ? "" : "s"} → fleet` : "Every print is queued"}
          </button>
        )}
        <button
          type="button"
          className="prj-btn"
          disabled={busy}
          onClick={() =>
            void run(
              () => projectsApi.update(pid, { name: null, due_date: null, status: ro ? "active" : "archived" }),
              ro ? "Reopened" : "Project archived · find it under Archived",
            ).then(() => !ro && nav("/projects"))
          }
        >
          {ro ? "Reopen" : "Archive"}
        </button>
        {p.status === "done" && (
          <button
            type="button"
            className="prj-btn"
            disabled={busy}
            onClick={() => void run(() => projectsApi.update(pid, { name: null, due_date: null, status: "archived" }), "Archived")}
          >
            Archive
          </button>
        )}
      </div>
      {error && <p className="prj-note prj-note--bad" role="alert">{error}</p>}
      {toast && <p className="prj-note prj-note--ok" role="status">{toast}</p>}
      {ro && (
        <p className="prj-note">
          {p.status === "done" ? "✓ Every print is done." : "Archived."} This is a read-only record
          {p.closed_at ? ` since ${new Date(p.closed_at).toLocaleDateString()}` : ""}. Reopen it to add copies or queue prints.
        </p>
      )}
      {swapAsk && (
        <div className="prj-note prj-note--warn" role="alert" style={{ display: "flex", flexWrap: "wrap", gap: 8, alignItems: "center" }}>
          <span style={{ flex: "1 1 300px" }}>⚠ {swapAsk}</span>
          <button
            type="button"
            className="prj-btn prj-btn--small"
            disabled={busy}
            onClick={() =>
              void run(async () => {
                const r = await projectsApi.printRemaining(pid, true);
                setSwapAsk(null);
                setToast(`${r.added} print${r.added === 1 ? "" : "s"} added to the fleet queue`);
              })
            }
          >
            Queue anyway
          </button>
          <button type="button" className="prj-btn prj-btn--small" onClick={() => setSwapAsk(null)}>
            Cancel
          </button>
        </div>
      )}

      <div>
        <div className="prj-bar" aria-hidden="true" style={{ height: 14 }}>
          <span className="prj-bar__done" style={{ width: `${pct(t.printed)}%` }} />
          <span className="prj-bar__run" style={{ width: `${pct(t.printing)}%` }} />
          <span className="prj-bar__queued" style={{ width: `${pct(t.queued)}%` }} />
        </div>
        <p className="prj-sub" style={{ marginTop: 6 }}>
          {progressText(t)} · {hours(t.secs_used)} printed of ~{hours(planned)} · {Math.round(t.grams_used)} g used
          {t.cost_used != null && p.currency ? ` · ${money(t.cost_used, p.currency)}` : ""}
          {p.status === "done" ? " · ✓ closed" : ""}
        </p>
      </div>

      <div className="pd-grid">
        <section aria-label="Plates" className="prj-sec">
          <div className="prj-sec__head">
            <h2 className="prj-sec__title">
              Plates · {p.plates.length} · {t.prints} prints
            </h2>
          </div>
          {p.plates.map((pl, i) => {
            const st = plateStatus(pl.counts, pl.live);
            const g = pl.grams.reduce((a, x) => a + x.grams, 0);
            const c = pl.counts;
            return (
              <div key={pl.id} className="pd-plate">
                <span className="tpl-plate__n">{i + 1}</span>
                <Thumb url={pl.file.thumb_url} />
                <span style={{ minWidth: 0 }}>
                  <b>{pl.name}</b>
                  <br />
                  <span className="prj-muted">
                    {hours(pl.file.print_time_secs)} · {Math.round(g)} g{c.copies > 1 ? ` each · ${c.printed} of ${c.copies} printed` : ""} ·{" "}
                    {kindLabel(pl.file.slot_kind, pl.file.slots.length)}
                  </span>
                  {pl.file.slot_kind === "change" && <span className="prj-flag">⚠ manual swaps (M600)</span>}
                  <span className="pd-plate__slots">
                    {[...new Set(pl.tools.map((x) => x.slot))].map((k) => (
                      <span key={k} className="pd-slotchip" title={k}>
                        <Dot hex={hex(k)} />
                        {k}
                      </span>
                    ))}
                  </span>
                </span>
                <span className="prj-step" role="group" aria-label={`Copies of ${pl.name}`}>
                  <button
                    type="button"
                    aria-label="Fewer copies"
                    disabled={ro || busy || c.copies <= pl.min_copies}
                    title={c.copies <= pl.min_copies ? `${c.printed} printed, ${c.printing} printing` : undefined}
                    onClick={() => void run(() => projectsApi.setCopies(pl.id, c.copies - 1))}
                  >
                    −
                  </button>
                  <output>× {c.copies}</output>
                  <button type="button" aria-label="More copies" disabled={ro || busy} onClick={() => void run(() => projectsApi.setCopies(pl.id, c.copies + 1))}>
                    +
                  </button>
                </span>
                <span>
                  {!ro && c.printed + c.printing < c.copies && (
                    <button
                      type="button"
                      className="prj-btn prj-btn--small"
                      title="Count one copy as printed (printed outside Foreman)"
                      disabled={busy}
                      onClick={() => void run(() => projectsApi.markDone(pl.id), `${pl.name}: one copy marked done`)}
                    >
                      Mark 1 done
                    </button>
                  )}
                </span>
                <span className={`pd-plate__status prj-tone--${st.tone}`}>{st.text}</span>
              </div>
            );
          })}
        </section>

        <div className="pd-side">
          <section aria-label="Materials" className="prj-sec">
            <h2 className="prj-sec__title">Materials</h2>
            {p.slots.map((s) => {
              const left = s.spool?.remaining_weight ?? 0;
              const free = left - s.reserved_elsewhere;
              const short = s.spool != null && s.need > free;
              const options = plan?.slots.find((x) => x.key === s.key)?.options ?? [];
              return (
                <div key={s.key} className="pd-mat">
                  <div className="pd-mat__head">
                    <span className="prj-mono" style={{ fontSize: 12 }}>
                      {s.key}
                    </span>
                    <Dot hex={s.spool?.color_hex ?? s.spool?.multi_color_hexes[0] ?? s.hex} big />
                    <span>{s.spool ? `#${s.spool.id} ${[s.spool.vendor, s.spool.material, s.spool.name].filter(Boolean).join(" ")}` : "No spool yet"}</span>
                    {!ro && (
                      <button type="button" className="prj-btn prj-btn--small" aria-expanded={swap === s.key} onClick={() => setSwap(swap === s.key ? null : s.key)}>
                        Swap
                      </button>
                    )}
                  </div>
                  <div className={short || !s.spool ? "prj-tone--warn" : "prj-tone--ok"} style={{ fontSize: 13 }}>
                    {!s.spool
                      ? `⚠ Needs ${Math.round(s.need)} g · nothing reserved`
                      : short
                        ? `⚠ Needs ${Math.round(s.need)} g · only ${Math.round(free)} g free · short ${Math.round(s.need - free)} g`
                        : `✓ Needs ${Math.round(s.need)} g · ${Math.round(left)} g left · ${Math.round(s.reserved)} g reserved`}
                  </div>
                  {swap === s.key && (
                    <div role="listbox" aria-label={`Choose spool for ${s.key}`} style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                      {!plan && <span className="prj-muted">Loading spools…</span>}
                      {options
                        .filter((o) => o.spool.id !== s.spool?.id)
                        .slice(0, 8)
                        .map((o) => {
                          // Free on this spool if the project's own reservation moved to it.
                          const f = fitText({ ...o, fits: o.free >= s.need }, s.need);
                          return (
                            <button
                              key={o.spool.id}
                              type="button"
                              role="option"
                              aria-selected={false}
                              className="prj-opt"
                              onClick={() =>
                                void run(() => projectsApi.setSlot(pid, s.key, o.spool.id), `${s.key} → #${o.spool.id} · reservation moved`).then(() => setSwap(null))
                              }
                            >
                              <Dot hex={o.spool.color_hex ?? o.spool.multi_color_hexes[0]} />
                              <span className="prj-opt__body">
                                #{o.spool.id} {[o.spool.vendor, o.spool.material, o.spool.name].filter(Boolean).join(" ")}
                                <br />
                                <span className={`prj-tone--${f.tone}`} style={{ fontSize: 12 }}>
                                  {Math.round(o.free)} g free · {f.text}
                                </span>
                              </span>
                            </button>
                          );
                        })}
                    </div>
                  )}
                </div>
              );
            })}
          </section>

          <section aria-label="Activity" className="prj-sec">
            <h2 className="prj-sec__title">Activity</h2>
            <div className="pd-activity">
              {p.events.map((e, i) => (
                <div key={i}>
                  <span>{when(e.at)}</span>
                  <span>{e.text}</span>
                </div>
              ))}
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}
