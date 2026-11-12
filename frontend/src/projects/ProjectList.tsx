/** Project list. Reference: docs/ui-ref/Projects.dc.html lines 47–71. */
import { useCallback, useEffect, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { hours, progressText, projectsApi, type ProjectSummary } from "../api/projects";
import { money } from "../api/inventory";
import { useEventStream } from "../hooks/useEventStream";
import { usePhone } from "../hooks/useMediaQuery";
import { EdgePanel, PhoneTitle } from "../ui";
import { Dot } from "./Thumb";

const TOPICS = ["queue", "job"];

export function dueLabel(p: { due_date: string | null; status: string; closed_at: string | null }): { text: string; tone: string } {
  if (p.status === "archived") return { text: "ARCHIVED", tone: "off" };
  if (p.status === "done") return { text: `DONE ${p.closed_at ? new Date(p.closed_at).toLocaleDateString(undefined, { day: "numeric", month: "short" }).toUpperCase() : ""}`.trim(), tone: "ok" };
  if (!p.due_date) return { text: "NO DATE", tone: "off" };
  const d = new Date(`${p.due_date}T00:00:00`);
  const days = Math.round((d.getTime() - new Date(new Date().toDateString()).getTime()) / 86400000);
  const text = `DUE ${d.toLocaleDateString(undefined, { day: "numeric", month: "short" }).toUpperCase()}`;
  return { text: days < 0 ? `OVERDUE · ${text.slice(4)}` : text, tone: days < 0 ? "bad" : days <= 3 ? "warn" : "off" };
}

export default function ProjectList() {
  const nav = useNavigate();
  const [rows, setRows] = useState<ProjectSummary[] | null>(null);
  const [filter, setFilter] = useState<"active" | "archived">("active");
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(() => {
    projectsApi.list().then(
      (r) => {
        setRows(r);
        setError(null);
      },
      (e) => setError(e instanceof Error ? e.message : String(e)),
    );
  }, []);
  useEffect(load, [load]);
  useEventStream(load, TOPICS);
  // Finished (done) and archived projects are read-only records under Archived.
  const phone = usePhone();
  const isActive = (p: ProjectSummary) => p.status === "active";
  const shown = (rows ?? []).filter((p) => (filter === "active") === isActive(p));
  const count = (f: "active" | "archived") => (rows ?? []).filter((p) => (f === "active") === isActive(p)).length;

  return (
    <div className="prj">
      <div className="prj-head">
        <div className="prj-head__main">
          <h1 className="prj-title phone-dup-title">Projects</h1>
          <p className="prj-sub">Due soonest first. Progress counts prints, not plates.</p>
        </div>
        <Link className="prj-btn" to="templates">
          Templates
        </Link>
        <Link className="prj-btn prj-btn--primary" to="new">
          New project
        </Link>
      </div>
      <PhoneTitle>
        Projects <span>· {filter === "active" ? "Active" : "Archived"}</span>
      </PhoneTitle>
      <EdgePanel side="left" label="Show projects" storageKey="projects.filter">
      <div role="group" aria-label="Show" className="prj-segs">
        {(
          [
            ["active", "Active"],
            ["archived", "Archived"],
          ] as const
        ).map(([id, label]) => (
          <button key={id} type="button" className="prj-seg" data-panel-close aria-pressed={filter === id} onClick={() => setFilter(id)}>
            {label} · {count(id)}
          </button>
        ))}
      </div>
      </EdgePanel>
      {error && <p className="prj-note prj-note--bad" role="alert">{error}</p>}
      {rows == null ? (
        <p className="prj-muted">Loading…</p>
      ) : shown.length === 0 ? (
        <p className="prj-muted">
          {filter === "active" ? (
            <>
              No active projects. <Link to="new">Start one from a template</Link>.
            </>
          ) : (
            "Nothing here."
          )}
        </p>
      ) : (
        <div className="prj-cards">
          {shown.map((p) => {
            const due = dueLabel(p);
            const t = p.totals;
            const pct = (n: number) => (t.prints ? (n / t.prints) * 100 : 0);
            return (
              phone ? (
                /* Phone (doc 06 §4): swatches, name, progress bar, "3 of 8"; nothing per plate. */
                <button key={p.id} type="button" className="prj-card prj-card--phone" onClick={() => nav(String(p.id))}>
                  <span className="prj-card__swatch" aria-hidden="true">
                    {p.swatches.slice(0, 4).map((h, i) => (
                      <Dot key={i} hex={h} />
                    ))}
                  </span>
                  <span className="prj-card__name">{p.name}</span>
                  <span className="prj-card__count">
                    {t.printed} of {t.prints}
                  </span>
                  <span className="prj-bar" aria-hidden="true">
                    <span className="prj-bar__done" style={{ width: `${pct(t.printed)}%` }} />
                    <span className="prj-bar__run" style={{ width: `${pct(t.printing)}%` }} />
                    <span className="prj-bar__queued" style={{ width: `${pct(t.queued)}%` }} />
                  </span>
                </button>
              ) : (
              <button key={p.id} type="button" className="prj-card" onClick={() => nav(String(p.id))}>
                <span className="prj-card__top">
                  <span className="prj-card__name">{p.name}</span>
                  <span className={`prj-chip prj-tone--${due.tone}`}>{due.text}</span>
                </span>
                <span className="prj-bar" aria-hidden="true">
                  <span className="prj-bar__done" style={{ width: `${pct(t.printed)}%` }} />
                  <span className="prj-bar__run" style={{ width: `${pct(t.printing)}%` }} />
                  <span className="prj-bar__queued" style={{ width: `${pct(t.queued)}%` }} />
                </span>
                <span className="prj-muted">{progressText(t)}</span>
                {!isActive(p) && (
                  <span className="prj-muted">
                    {[
                      t.secs_used ? hours(t.secs_used) : null,
                      t.grams_used ? `${Math.round(t.grams_used)} g` : null,
                      t.cost_used != null && p.currency ? money(t.cost_used, p.currency) : null,
                    ]
                      .filter(Boolean)
                      .join(" · ") || "No print records"}
                  </span>
                )}
                <span className="prj-swatches">
                  {p.swatches.map((h, i) => (
                    <Dot key={i} hex={h} />
                  ))}
                  {p.template_name && <span className="prj-muted" style={{ marginLeft: 6 }}>{p.template_name}</span>}
                </span>
              </button>
              )
            );
          })}
        </div>
      )}
      {phone && (
        <div className="prj-phone-add">
          <Link className="ui-list-add" to="new">
            + New project
          </Link>
          <Link className="ui-list-add" to="templates">
            Templates
          </Link>
        </div>
      )}
    </div>
  );
}
