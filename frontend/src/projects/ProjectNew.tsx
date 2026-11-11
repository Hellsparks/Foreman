/**
 * New project (tasks 5.1, 5.2). Reference: docs/ui-ref/ProjectNew.dc.html lines 1–178
 * + renderVals. From a template, or from loose Library files whose filaments are
 * grouped into shared colour slots (same material + colour), split or merged by hand.
 * Spools are picked from the sliced material by default ("PLA only, as sliced").
 */
import { useEffect, useMemo, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router-dom";
import type { LibraryEntry } from "../api/library";
import {
  fitText,
  hours,
  isManualSwaps,
  kindLabel,
  MAX_COPIES,
  MAX_SETS,
  projectsApi,
  type ProjectPlan,
  type SpoolOption,
  type TemplateView,
} from "../api/projects";
import LibraryPicker from "./LibraryPicker";
import { choicesFor, entrySlots, groupMaterials, groupSlots, memberId, type Assign, type PickedFile } from "./grouping";
import Thumb, { Dot } from "./Thumb";

const SHOW = 6;

function spoolLabel(o: SpoolOption): string {
  return `#${o.spool.id} ${[o.spool.vendor, o.spool.material, o.spool.name].filter(Boolean).join(" ")}`;
}

/** One colour slot to pick a spool for, from either source. */
interface SlotRow {
  key: string;
  label: string | null;
  hex: string | null;
  materials: string[];
  plates: string;
  need: number;
  keepColour: boolean;
  options: SpoolOption[];
}

type LooseFile = PickedFile & { thumb_url: string | null; print_time_secs: number | null };

export default function ProjectNew() {
  const nav = useNavigate();
  const [params] = useSearchParams();
  const [source, setSource] = useState<"template" | "files">(params.get("source") === "files" ? "files" : "template");
  const [templates, setTemplates] = useState<TemplateView[] | null>(null);
  const [tpl, setTpl] = useState<number | null>(params.get("template") ? Number(params.get("template")) : null);
  const [sets, setSets] = useState(1);
  const [name, setName] = useState<string | null>(null);
  const [due, setDue] = useState("");
  const [plan, setPlan] = useState<ProjectPlan | null>(null);
  const [files, setFiles] = useState<LooseFile[]>([]);
  const [assign, setAssign] = useState<Assign>({});
  const [fileOptions, setFileOptions] = useState<Record<string, SpoolOption[]>>({});
  const [picker, setPicker] = useState(false);
  const [pick, setPick] = useState<Record<string, number | null>>({});
  const [showAll, setShowAll] = useState<Record<string, boolean>>({});
  const [search, setSearch] = useState<Record<string, string>>({});
  const [confirmSwaps, setConfirmSwaps] = useState(false);
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

  // Template source: the server's plan (needs + ranked spools).
  useEffect(() => {
    if (source !== "template" || tpl == null) return;
    let live = true;
    projectsApi.plan(tpl, sets).then(
      (p) => live && setPlan(p),
      (e) => live && setError(e instanceof Error ? e.message : String(e)),
    );
    return () => {
      live = false;
    };
  }, [source, tpl, sets]);

  // Loose files: group filaments, then rank spools per group.
  const groups = useMemo(() => groupSlots(files, assign), [files, assign]);
  const needsKey = JSON.stringify(groups.map((g) => [g.key, g.perSet * sets, groupMaterials(g), g.hex]));
  useEffect(() => {
    if (source !== "files" || groups.length === 0) return;
    let live = true;
    const t = setTimeout(() => {
      projectsApi
        .spoolOptions(groups.map((g) => ({ key: g.key, need: g.perSet * sets, materials: groupMaterials(g), hex: g.hex })))
        .then(
          (r) => live && setFileOptions(Object.fromEntries(r.map((x) => [x.key, x.options]))),
          (e) => live && setError(e instanceof Error ? e.message : String(e)),
        );
    }, 200);
    return () => {
      live = false;
      clearTimeout(t);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [source, needsKey]);

  const slots: SlotRow[] = useMemo(() => {
    if (source === "template")
      return (plan?.slots ?? []).map((s) => ({
        key: s.key,
        label: s.label,
        hex: s.hex,
        materials: s.materials,
        plates: s.plates.join(", "),
        need: s.need,
        keepColour: !s.allow_swap,
        options: s.options,
      }));
    return groups.map((g) => ({
      key: g.key,
      label: null,
      hex: g.hex,
      materials: groupMaterials(g),
      plates: g.members.map((m) => `${m.file.replace(/\.(gcode|bgcode|3mf)$/i, "")} ${m.slot.label}${m.copies > 1 ? ` ×${m.copies}` : ""}`).join(", "),
      need: g.perSet * sets,
      keepColour: false,
      options: fileOptions[g.key] ?? [],
    }));
  }, [source, plan, groups, fileOptions, sets]);

  // Default pick per slot: the best option of the sliced material.
  useEffect(() => {
    setPick((cur) => {
      const next: Record<string, number | null> = {};
      let changed = false;
      for (const s of slots) {
        const keep = cur[s.key] !== undefined && (cur[s.key] === null || s.options.some((o) => o.spool.id === cur[s.key]));
        next[s.key] = keep ? cur[s.key] : s.options[0]?.spool.id ?? null;
        if (next[s.key] !== cur[s.key]) changed = true;
      }
      return changed || Object.keys(cur).length !== slots.length ? next : cur;
    });
  }, [slots]);

  const t = templates?.find((x) => x.id === tpl) ?? null;
  const baseName = (n: string) => n.replace(/\.(gcode|bgcode|3mf)$/i, "").replace(/\.gcode$/i, "").replace(/[_-]+/g, " ");
  const defaultName =
    source === "template"
      ? t
        ? `${t.template.name}${sets > 1 ? ` ×${sets}` : ""}`
        : ""
      : files.length
        ? `${baseName(files[0].name)}${files.length > 1 ? ` + ${files.length - 1} more` : ""}`
        : "";
  const prints = source === "template" ? plan?.prints ?? 0 : files.reduce((a, f) => a + f.copies, 0) * sets;
  const total = slots.reduce((a, s) => a + s.need, 0);
  const swapFiles =
    source === "template"
      ? (t?.files ?? []).filter((f) => f.slot_kind === "change").map((f) => f.file_name)
      : files.filter((f) => f.slot_kind === "change").map((f) => f.name);
  const ready = source === "template" ? !!plan : files.length > 0 && slots.length > 0;

  const summary = useMemo(() => {
    if (!slots.length) return null;
    let short = 0;
    let unset = 0;
    let warn = "";
    for (const s of slots) {
      const o = s.options.find((x) => x.spool.id === pick[s.key]);
      if (!o) {
        unset++;
        continue;
      }
      if (!o.fits) short += s.need - Math.max(0, o.free);
      else if (!o.material_ok) warn = `${s.key}: ${o.spool.material ?? "?"} spool, sliced for ${s.materials.join("/")}.`;
      else if (s.keepColour && (o.colour_distance ?? 0) > 60) warn = `${s.key} should keep its colour (template says no swap).`;
    }
    if (unset) return { tone: "warn", text: `⚠ ${unset} slot${unset > 1 ? "s" : ""} without a spool: nothing is reserved for ${unset > 1 ? "them" : "it"}.` };
    if (short) return { tone: "bad", text: `✖ Not enough filament: short ${Math.round(short)} g. Pick another spool, or create anyway and swap later.` };
    if (warn) return { tone: "warn", text: `⚠ ${warn}` };
    return { tone: "ok", text: "✓ Every colour has enough free filament." };
  }, [slots, pick]);

  const create = async (queueAll: boolean, acceptSwaps = false) => {
    if (queueAll && swapFiles.length && !acceptSwaps) {
      setConfirmSwaps(true);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const common = {
        sets,
        name: (name ?? defaultName).trim() || null,
        due_date: due || null,
        queue_all: queueAll,
        accept_manual_swaps: acceptSwaps,
      };
      const v =
        source === "template"
          ? await projectsApi.create({
              ...common,
              template_id: tpl ?? 0,
              spools: slots.map((s) => ({ key: s.key, spool_id: pick[s.key] ?? null })),
            })
          : await projectsApi.fromFiles({
              ...common,
              files: files.map((f) => ({ file_id: f.file_id, copies: f.copies, name: null })),
              slots: groups.map((g) => ({
                key: g.key,
                label: null,
                hex: g.hex,
                spool_id: pick[g.key] ?? null,
                members: g.members.map((m) => ({ file_id: m.file_id, slot: m.slot.index })),
              })),
            });
      nav(`/projects/${v.id}`);
    } catch (e) {
      if (isManualSwaps(e)) setConfirmSwaps(true);
      setError(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  };

  const addFiles = (es: LibraryEntry[]) => {
    setFiles([
      ...files,
      ...es.map((e) => ({
        file_id: e.id,
        name: e.name,
        slot_kind: e.slot_kind ?? "tool",
        slots: entrySlots(e),
        copies: 1,
        thumb_url: e.thumb_url,
        print_time_secs: e.print_time_secs,
      })),
    ]);
    setPicker(false);
  };
  const setCopies = (id: number, c: number) => setFiles(files.map((f) => (f.file_id === id ? { ...f, copies: Math.max(1, Math.min(MAX_COPIES, c)) } : f)));

  return (
    <div className="prj">
      <Link className="prj-back" to="/projects">
        ← Projects
      </Link>
      <div>
        <h1 className="prj-title">New project</h1>
        <p className="prj-sub">Pick what to print, then which spool feeds each colour. Grams are reserved when you create the project.</p>
      </div>
      {error && <p className="prj-note prj-note--bad" role="alert">{error}</p>}

      <section aria-label="Source" className="prj-sec">
        <h2 className="prj-sec__title">1 · What to print</h2>
        <div role="radiogroup" aria-label="Source" className="prj-segs">
          {(
            [
              ["template", "From a template"],
              ["files", "From Library files"],
            ] as const
          ).map(([id, label]) => (
            <button key={id} type="button" role="radio" aria-checked={source === id} className="prj-seg" onClick={() => setSource(id)}>
              {label}
            </button>
          ))}
        </div>
        {source === "template" ? (
          templates == null ? (
            <p className="prj-muted">Loading…</p>
          ) : templates.length === 0 ? (
            <p className="prj-muted">
              No ready template yet. <Link to="/projects/templates/new">Make one</Link>, or start from Library files.
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
                      {x.template.plates.length} plates · {x.prints_per_set} prints · {x.template.slots.length} colour{x.template.slots.length === 1 ? "" : "s"} ·{" "}
                      {hours(x.print_time_secs)}
                    </span>
                  </span>
                </button>
              ))}
            </div>
          )
        ) : (
          <>
            <p className="prj-muted" style={{ margin: 0 }}>
              Each file becomes a plate. Foreman reads which filament each tool, AMS tray or colour change uses from the slicer data in the file, and files
              that use the same material and colour share one colour slot.
            </p>
            <div className="np-files">
              {files.map((f) => (
                <div key={f.file_id} className="np-file">
                  <Thumb url={f.thumb_url} />
                  <span style={{ minWidth: 0 }}>
                    <b>{f.name}</b>
                    <br />
                    <span className="prj-muted">
                      {hours(f.print_time_secs)} · {kindLabel(f.slot_kind, f.slots.length)}
                      {f.copies > 1 ? ` · ×${f.copies} = ${f.copies} prints per set` : ""}
                    </span>
                    <span className="np-file__chips">
                      {f.slots.map((s) => (
                        <span key={s.index} className="pd-slotchip">
                          <Dot hex={s.colour} />
                          {s.label} {s.material ?? ""}
                        </span>
                      ))}
                      {f.slot_kind === "change" && <span className="prj-flag">⚠ manual swaps (M600)</span>}
                    </span>
                  </span>
                  <span className="prj-step" role="group" aria-label={`Copies of ${f.name}`}>
                    <button type="button" aria-label="Fewer copies" disabled={f.copies <= 1} onClick={() => setCopies(f.file_id, f.copies - 1)}>
                      −
                    </button>
                    <output>× {f.copies}</output>
                    <button type="button" aria-label="More copies" disabled={f.copies >= MAX_COPIES} onClick={() => setCopies(f.file_id, f.copies + 1)}>
                      +
                    </button>
                  </span>
                  <button type="button" className="prj-btn prj-btn--small" aria-label={`Remove ${f.name}`} onClick={() => setFiles(files.filter((x) => x.file_id !== f.file_id))}>
                    ✕
                  </button>
                </div>
              ))}
            </div>
            <div>
              <button type="button" className="prj-btn" onClick={() => setPicker(true)}>
                {files.length ? "+ Add more files…" : "Pick files from the Library…"}
              </button>
            </div>
            {files.length > 0 && (
              <div className="np-map">
                <b style={{ fontSize: 13 }}>Which slot each filament uses</b>
                {files.flatMap((f) =>
                  f.slots.map((s) => {
                    const id = memberId(f.file_id, s.index);
                    const cur = groups.find((g) => g.members.some((m) => m.file_id === f.file_id && m.slot.index === s.index))?.key ?? "";
                    return (
                      <div key={id} className="np-map__row">
                        <span>
                          <b>{f.name}</b> · {s.label}
                        </span>
                        <span className="prj-muted" style={{ display: "inline-flex", gap: 6, alignItems: "center" }}>
                          <Dot hex={s.colour} />
                          slicer says {[s.material, s.colour].filter(Boolean).join(" ")}
                          {s.grams != null ? ` · ${Math.round(s.grams * 10) / 10} g` : ""}
                        </span>
                        <span aria-hidden="true">→</span>
                        <select
                          className="prj-input"
                          aria-label={`Slot for ${f.name} ${s.label}`}
                          value={cur}
                          onChange={(e) => setAssign({ ...assign, [id]: e.target.value })}
                        >
                          {choicesFor(groups, f, s).map((o) => (
                            <option key={o.value} value={o.value}>
                              {o.label}
                            </option>
                          ))}
                        </select>
                      </div>
                    );
                  }),
                )}
                <span className="prj-muted">
                  Pick “Own slot” to give a filament its own spool (e.g. the same white, but you want a different spool), or move it into another slot to merge
                  them.
                </span>
              </div>
            )}
          </>
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
        {prints > 0 && <span className="prj-muted">{prints} prints in total.</span>}
      </section>

      <section aria-label="Colours" className="prj-sec">
        <h2 className="prj-sec__title">3 · Spool for each colour</h2>
        <p className="prj-muted" style={{ margin: 0 }}>
          Free = grams left minus what other projects and queued jobs already reserved.
        </p>
        {slots.length === 0 && <p className="prj-muted">{source === "files" ? "Pick files first." : "Pick a template first."}</p>}
        {slots.map((s) => {
          const all = !!showAll[s.key] || s.materials.length === 0;
          const pool = all ? s.options : s.options.filter((o) => o.material_ok);
          const q = (search[s.key] ?? "").toLowerCase().replace(/#/g, "").split(/\s+/).filter(Boolean);
          const hits = pool.filter((o) => {
            if (!q.length) return true;
            const hay = [`#${o.spool.id}`, o.spool.vendor, o.spool.material, o.spool.name, o.spool.color_hex, o.spool.location].filter(Boolean).join(" ").toLowerCase().replace(/#/g, "");
            return q.every((w) => hay.includes(w));
          });
          const chosen = s.options.find((o) => o.spool.id === pick[s.key]);
          const list = hits.slice(0, SHOW);
          if (chosen && !list.includes(chosen)) list.unshift(chosen);
          const mat = s.materials.join("/");
          return (
            <div key={s.key} className="np-slot">
              <div className="np-slot__head">
                <Dot hex={s.hex} big />
                <b>{s.key}</b>
                {mat && <span className="np-mat">{mat}</span>}
                <span className="prj-muted">
                  {[s.label].filter(Boolean).join(" · ")}
                  {s.label ? " · " : ""}used on {s.plates}
                  {s.keepColour ? " · keep colour" : ""}
                </span>
                <span style={{ flex: 1 }} />
                <span className="prj-mono">needs {Math.round(s.need)} g</span>
              </div>
              {s.materials.length > 0 && (
                <div className="np-pool">
                  <span>
                    {all
                      ? `Showing all ${s.options.length} spools · sliced for ${mat}`
                      : `Showing ${pool.length} of ${s.options.length} spools · ${mat} only, as sliced`}
                  </span>
                  <button type="button" className="prj-btn prj-btn--small" onClick={() => setShowAll({ ...showAll, [s.key]: !showAll[s.key] })}>
                    {all ? `Only ${mat}` : "Show all materials"}
                  </button>
                </div>
              )}
              {chosen && !chosen.material_ok && (
                <p className="prj-note prj-note--warn">
                  ⚠ {chosen.spool.material ?? "Unknown material"} chosen, file sliced for {mat} — temperatures and shrinkage will differ.
                </p>
              )}
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
              {!all && pool.length === 0 && <span className="prj-muted">No {mat} spool in stock. Show all materials to pick another.</span>}
            </div>
          );
        })}
      </section>

      {confirmSwaps && (
        <div className="prj-note prj-note--warn" role="alert" style={{ display: "flex", flexWrap: "wrap", gap: 8, alignItems: "center" }}>
          <span style={{ flex: "1 1 300px" }}>
            ⚠ {swapFiles.join(", ")} change{swapFiles.length === 1 ? "s" : ""} filament by hand (M600): someone has to swap spools during each print. Queue
            anyway?
          </span>
          <button type="button" className="prj-btn prj-btn--small" disabled={busy} onClick={() => void create(true, true)}>
            Queue anyway
          </button>
          <button type="button" className="prj-btn prj-btn--small" onClick={() => setConfirmSwaps(false)}>
            Cancel
          </button>
        </div>
      )}

      <div className="np-foot">
        <div className="np-foot__text">
          <b>
            {(name ?? defaultName) || "—"} · {sets} set{sets > 1 ? "s" : ""} · {prints} prints · {Math.round(total)} g
          </b>
          <br />
          {summary && <span className={`prj-tone--${summary.tone}`}>{summary.text}</span>}
          {swapFiles.length > 0 && (
            <>
              <br />
              <span className="prj-tone--warn">⚠ Manual filament swaps: {swapFiles.join(", ")}</span>
            </>
          )}
        </div>
        <button type="button" className="prj-btn" disabled={busy || !ready} onClick={() => void create(false)}>
          Create project
        </button>
        <button type="button" className="prj-btn prj-btn--primary" disabled={busy || !ready} onClick={() => void create(true)}>
          Create and queue all
        </button>
      </div>

      {picker && (
        <LibraryPicker
          title="Pick files for the project"
          verb="file"
          taken={new Set(files.map((f) => f.file_id))}
          takenLabel="Already picked"
          onClose={() => setPicker(false)}
          onAdd={addFiles}
        />
      )}
    </div>
  );
}
