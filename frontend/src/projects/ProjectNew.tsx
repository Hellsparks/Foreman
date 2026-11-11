/**
 * New project from a template (task 5.1). Reference: docs/ui-ref/ProjectNew.dc.html
 * lines 1–178 + renderVals. "From Library files" is next session.
 */
import { useEffect, useMemo, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router-dom";
import {
  fitText,
  hours,
  MAX_SETS,
  projectsApi,
  type ProjectPlan,
  type SpoolOption,
  type TemplateView,
} from "../api/projects";
import { Dot } from "./Thumb";

const SHOW = 6;

function spoolLabel(o: SpoolOption): string {
  return `#${o.spool.id} ${[o.spool.vendor, o.spool.material, o.spool.name].filter(Boolean).join(" ")}`;
}

export default function ProjectNew() {
  const nav = useNavigate();
  const [params] = useSearchParams();
  const [templates, setTemplates] = useState<TemplateView[] | null>(null);
  const [tpl, setTpl] = useState<number | null>(params.get("template") ? Number(params.get("template")) : null);
  const [sets, setSets] = useState(1);
  const [name, setName] = useState<string | null>(null);
  const [due, setDue] = useState("");
  const [plan, setPlan] = useState<ProjectPlan | null>(null);
  const [pick, setPick] = useState<Record<string, number | null>>({});
  const [search, setSearch] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    projectsApi.templates().then(
      (all) => {
        const ready = all.filter((t) => t.problems.length === 0);
        setTemplates(ready);
        setTpl((cur) => (cur != null && ready.some((t) => t.id === cur) ? cur : ready[0]?.id ?? null));
      },
      (e) => setError(e instanceof Error ? e.message : String(e)),
    );
  }, []);

  useEffect(() => {
    if (tpl == null) return;
    let live = true;
    projectsApi.plan(tpl, sets).then(
      (p) => {
        if (!live) return;
        setPlan(p);
        // Default pick per slot: the best option (server sorts material, fit, colour).
        setPick((cur) => {
          const next: Record<string, number | null> = {};
          for (const s of p.slots) next[s.key] = cur[s.key] !== undefined && s.options.some((o) => o.spool.id === cur[s.key]) ? cur[s.key] : s.options[0]?.spool.id ?? null;
          return next;
        });
      },
      (e) => live && setError(e instanceof Error ? e.message : String(e)),
    );
    return () => {
      live = false;
    };
  }, [tpl, sets]);

  const t = templates?.find((x) => x.id === tpl) ?? null;
  const defaultName = t ? `${t.template.name}${sets > 1 ? ` ×${sets}` : ""}` : "";
  const total = plan?.slots.reduce((a, s) => a + s.need, 0) ?? 0;
  const summary = useMemo(() => {
    if (!plan) return null;
    let short = 0;
    let unset = 0;
    let warn = "";
    for (const s of plan.slots) {
      const o = s.options.find((x) => x.spool.id === pick[s.key]);
      if (!o) {
        unset++;
        continue;
      }
      if (!o.fits) short += s.need - Math.max(0, o.free);
      else if (!o.material_ok) warn = `${s.key}: ${o.spool.material ?? "?"} spool, plates were sliced for ${s.materials.join("/")}.`;
      else if (!s.allow_swap && (o.colour_distance ?? 0) > 60) warn = `${s.key} should keep its colour (template says no swap).`;
    }
    if (unset) return { tone: "warn", text: `⚠ ${unset} slot${unset > 1 ? "s" : ""} without a spool: nothing is reserved for ${unset > 1 ? "them" : "it"}.` };
    if (short) return { tone: "bad", text: `✖ Not enough filament: short ${Math.round(short)} g. Pick another spool, or create anyway and swap later.` };
    if (warn) return { tone: "warn", text: `⚠ ${warn}` };
    return { tone: "ok", text: "✓ Every colour has enough free filament." };
  }, [plan, pick]);

  const create = async (queueAll: boolean) => {
    if (!tpl || !plan) return;
    setBusy(true);
    setError(null);
    try {
      const v = await projectsApi.create({
        template_id: tpl,
        sets,
        name: (name ?? defaultName).trim() || null,
        due_date: due || null,
        spools: plan.slots.map((s) => ({ key: s.key, spool_id: pick[s.key] ?? null })),
        queue_all: queueAll,
      });
      nav(`/projects/${v.id}`);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  };

  return (
    <div className="prj">
      <Link className="prj-back" to="/projects">
        ← Projects
      </Link>
      <div>
        <h1 className="prj-title">New project</h1>
        <p className="prj-sub">Pick a template, then which spool feeds each colour. Grams are reserved when you create the project.</p>
      </div>
      {error && <p className="prj-note prj-note--bad" role="alert">{error}</p>}

      <section aria-label="Source" className="prj-sec">
        <h2 className="prj-sec__title">1 · What to print</h2>
        <div role="radiogroup" aria-label="Source" className="prj-segs">
          <button type="button" role="radio" aria-checked="true" className="prj-seg">
            From a template
          </button>
          <button type="button" role="radio" aria-checked="false" className="prj-seg" disabled title="Projects from Library files come next">
            From Library files
          </button>
        </div>
        {templates == null ? (
          <p className="prj-muted">Loading…</p>
        ) : templates.length === 0 ? (
          <p className="prj-muted">
            No ready template yet. <Link to="/projects/templates/new">Make one</Link>.
          </p>
        ) : (
          <div role="radiogroup" aria-label="Template" className="np-templates">
            {templates.map((x) => (
              <button key={x.id} type="button" role="radio" aria-checked={x.id === tpl} className="prj-opt" onClick={() => setTpl(x.id)}>
                <span className="prj-swatches">
                  {x.template.slots.map((s) => (
                    <Dot key={s.key} hex={s.hex} />
                  ))}
                </span>
                <span className="prj-opt__body">
                  <b>{x.template.name}</b>
                  <br />
                  <span className="prj-muted">
                    {x.template.plates.length} plates · {x.prints_per_set} prints · {x.template.slots.length} colour{x.template.slots.length === 1 ? "" : "s"} · {hours(x.print_time_secs)}
                  </span>
                </span>
              </button>
            ))}
          </div>
        )}
      </section>

      <section aria-label="Details" className="prj-sec">
        <h2 className="prj-sec__title">2 · Details</h2>
        <div className="np-row">
          <label className="prj-field" style={{ flex: "1 1 260px" }}>
            <span className="prj-field__label">Name</span>
            <input className="prj-input" value={name ?? defaultName} onChange={(e) => setName(e.target.value)} />
          </label>
          <label className="prj-field">
            <span className="prj-field__label">
              Due date <span className="prj-muted">(optional)</span>
            </span>
            <input className="prj-input" type="date" value={due} onChange={(e) => setDue(e.target.value)} />
          </label>
          <div className="prj-field">
            <span className="prj-field__label">How many sets</span>
            <span className="prj-step">
              <button type="button" aria-label="Fewer sets" disabled={sets <= 1} onClick={() => setSets(sets - 1)}>
                −
              </button>
              <output>{sets}</output>
              <button type="button" aria-label="More sets" disabled={sets >= MAX_SETS} onClick={() => setSets(sets + 1)}>
                +
              </button>
            </span>
          </div>
        </div>
        {plan && <span className="prj-muted">{plan.prints} prints in total ({t?.prints_per_set} per set × {sets}).</span>}
      </section>

      <section aria-label="Colours" className="prj-sec">
        <h2 className="prj-sec__title">3 · Spool for each colour</h2>
        <p className="prj-muted" style={{ margin: 0 }}>
          Free = grams left minus what other projects and queued jobs already reserved.
        </p>
        {plan?.slots.map((s) => {
          const q = (search[s.key] ?? "").toLowerCase().replace(/#/g, "").split(/\s+/).filter(Boolean);
          const hits = s.options.filter((o) => {
            if (!q.length) return true;
            const hay = [`#${o.spool.id}`, o.spool.vendor, o.spool.material, o.spool.name, o.spool.color_hex, o.spool.location].filter(Boolean).join(" ").toLowerCase().replace(/#/g, "");
            return q.every((w) => hay.includes(w));
          });
          const chosen = s.options.find((o) => o.spool.id === pick[s.key]);
          const list = hits.slice(0, SHOW);
          if (chosen && !list.includes(chosen)) list.unshift(chosen);
          return (
            <div key={s.key} className="np-slot">
              <div className="np-slot__head">
                <Dot hex={s.hex} big />
                <b>{s.key}</b>
                <span className="prj-muted">
                  {[s.label, s.materials.join("/")].filter(Boolean).join(" · ")} · used on {s.plates.join(", ")}
                  {!s.allow_swap ? " · keep colour" : ""}
                </span>
                <span style={{ flex: 1 }} />
                <span className="prj-mono">needs {Math.round(s.need)} g</span>
              </div>
              <input
                type="search"
                className="prj-input"
                aria-label={`Search spools for ${s.key}`}
                placeholder="Search: #id, vendor, colour, hex…"
                value={search[s.key] ?? ""}
                onChange={(e) => setSearch({ ...search, [s.key]: e.target.value })}
              />
              <div role="radiogroup" aria-label={`Spool for ${s.key}`} className="np-opts">
                {list.map((o) => {
                  const f = fitText(o, s.need);
                  return (
                    <button key={o.spool.id} type="button" role="radio" aria-checked={pick[s.key] === o.spool.id} className="prj-opt" onClick={() => setPick({ ...pick, [s.key]: o.spool.id })}>
                      <Dot hex={o.spool.color_hex ?? o.spool.multi_color_hexes[0]} big />
                      <span className="prj-opt__body">
                        <b>{spoolLabel(o)}</b>
                        <br />
                        <span className="prj-muted">
                          {[o.loaded.length ? `Loaded · ${o.loaded.join(", ")}` : o.spool.location, `${Math.round(o.spool.remaining_weight ?? 0)} g left`, o.reserved ? `${Math.round(o.reserved)} g reserved elsewhere` : null]
                            .filter(Boolean)
                            .join(" · ")}
                        </span>
                        <br />
                        <span className={`prj-tone--${f.tone}`} style={{ fontSize: 12 }}>
                          {f.text}
                        </span>
                      </span>
                    </button>
                  );
                })}
                <button type="button" role="radio" aria-checked={pick[s.key] == null} className="prj-opt" onClick={() => setPick({ ...pick, [s.key]: null })}>
                  <Dot hex={null} big />
                  <span className="prj-opt__body">
                    <b>Decide later</b>
                    <br />
                    <span className="prj-muted">Nothing reserved for {s.key}</span>
                  </span>
                </button>
              </div>
              {hits.length > SHOW && <span className="prj-muted">{hits.length - SHOW} more match; search to narrow.</span>}
            </div>
          );
        })}
      </section>

      <div className="np-foot">
        <div className="np-foot__text">
          <b>
            {t?.template.name ?? "—"} · {sets} set{sets > 1 ? "s" : ""} · {plan?.prints ?? 0} prints · {Math.round(total)} g
          </b>
          <br />
          {summary && <span className={`prj-tone--${summary.tone}`}>{summary.text}</span>}
        </div>
        <button type="button" className="prj-btn" disabled={busy || !plan} onClick={() => void create(false)}>
          Create project
        </button>
        <button type="button" className="prj-btn prj-btn--primary" disabled={busy || !plan} onClick={() => void create(true)}>
          Create and queue all
        </button>
      </div>
    </div>
  );
}
