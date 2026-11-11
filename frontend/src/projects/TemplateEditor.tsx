/**
 * Templates: list + editor (task 5.1). Reference: docs/ui-ref/TemplateEditor.dc.html
 * lines 1–207, behaviour in renderVals (208+).
 */
import { useCallback, useEffect, useMemo, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { lib, type LibraryEntry } from "../api/library";
import {
  hours,
  MAX_COPIES,
  nextSlotKey,
  printsPerSet,
  projectsApi,
  renameSlot,
  setPlateSlot,
  slotKey,
  slotOf,
  type PlateFile,
  type TemplateForm,
  type TemplatePlate,
  type TemplateView,
} from "../api/projects";
import Thumb, { Dot } from "./Thumb";

const EMPTY: TemplateForm = { name: "", category: null, notes: null, thumb_file_id: null, slots: [], plates: [] };

function fileFacts(e: LibraryEntry): PlateFile {
  return { file_id: e.id, file_name: e.name, thumb_url: e.thumb_url, print_time_secs: e.print_time_secs, tools: e.tools, missing: false };
}

function plateMeta(f: PlateFile | undefined): string {
  if (!f) return "";
  if (f.missing) return "file missing";
  const g = f.tools.reduce((a, t) => a + (t.grams ?? 0), 0);
  const tools = f.tools.length > 1 ? ` · ${f.tools.length} tools` : "";
  return [hours(f.print_time_secs), g ? `${Math.round(g)} g` : null].filter(Boolean).join(" · ") + tools;
}

// ── Template list ────────────────────────────────────────────────────────────

export function TemplateList() {
  const [rows, setRows] = useState<TemplateView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    projectsApi.templates().then(setRows, (e) => setError(e instanceof Error ? e.message : String(e)));
  }, []);
  return (
    <div className="prj">
      <Link className="prj-back" to="..">
        ← Projects
      </Link>
      <div className="prj-head">
        <div className="prj-head__main">
          <h1 className="prj-title">Templates</h1>
          <p className="prj-sub">Reusable plates + colour slots. Start a project from one.</p>
        </div>
        <Link className="prj-btn prj-btn--primary" to="new">
          New template
        </Link>
      </div>
      {error && <p className="prj-note prj-note--bad" role="alert">{error}</p>}
      <section className="prj-sec">
        {rows == null && <p className="prj-muted">Loading…</p>}
        {rows?.length === 0 && <p className="prj-muted">No templates yet.</p>}
        <div className="tpl-list">
          {rows?.map((t) => (
            <div key={t.id} className="tpl-list__row">
              <Thumb url={t.template.thumb_file_id != null ? t.files.find((f) => f.file_id === t.template.thumb_file_id)?.thumb_url ?? t.files[0]?.thumb_url : t.files[0]?.thumb_url} />
              <span style={{ flex: 1, minWidth: 0 }}>
                <Link to={String(t.id)}>
                  <b>{t.template.name}</b>
                </Link>
                <br />
                <span className="prj-muted">
                  {[t.template.category, `${t.template.plates.length} plates`, `${t.prints_per_set} prints per set`, `${t.template.slots.length} colours`, hours(t.print_time_secs)]
                    .filter(Boolean)
                    .join(" · ")}
                </span>
              </span>
              <span className="prj-swatches">
                {t.template.slots.map((s) => (
                  <Dot key={s.key} hex={s.hex} />
                ))}
              </span>
              {t.problems.length ? (
                <span className="prj-chip prj-tone--warn">DRAFT</span>
              ) : (
                <Link className="prj-btn prj-btn--small" to={`../new?template=${t.id}`}>
                  Start project
                </Link>
              )}
            </div>
          ))}
        </div>
      </section>
    </div>
  );
}

// ── Editor ───────────────────────────────────────────────────────────────────

export default function TemplateEditor() {
  const { id } = useParams();
  const nav = useNavigate();
  const tplId = id != null && id !== "new" ? Number(id) : null;
  const [form, setForm] = useState<TemplateForm>(EMPTY);
  const [files, setFiles] = useState<Map<number, PlateFile>>(new Map());
  const [problems, setProblems] = useState<string[]>([]);
  const [dirty, setDirty] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [categories, setCategories] = useState<string[]>([]);

  const adopt = useCallback((v: TemplateView) => {
    setForm(v.template);
    setFiles(new Map(v.files.map((f) => [f.file_id, f])));
    setProblems(v.problems);
    setDirty(false);
  }, []);

  useEffect(() => {
    projectsApi.templates().then(
      (all) => setCategories([...new Set(all.map((t) => t.template.category).filter((c): c is string => !!c))]),
      () => {},
    );
    if (tplId == null) return;
    projectsApi.template(tplId).then(adopt, (e) => setError(e instanceof Error ? e.message : String(e)));
  }, [tplId, adopt]);

  const edit = (f: TemplateForm) => {
    setForm(f);
    setDirty(true);
    setNote(null);
  };
  const setPlate = (i: number, p: TemplatePlate) => edit({ ...form, plates: form.plates.map((x, j) => (j === i ? p : x)) });

  const addPlate = (e: LibraryEntry) => {
    const f = fileFacts(e);
    setFiles(new Map(files).set(e.id, f));
    let next = { ...form };
    const nTools = Math.max(1, f.tools.length);
    // First plate: create one slot per tool so the matrix starts filled in.
    while (next.slots.length < nTools && next.plates.length === 0) {
      const t = f.tools[next.slots.length];
      next = {
        ...next,
        slots: [
          ...next.slots,
          { key: nextSlotKey(next), hex: t?.colour?.replace(/^#/, "") ?? null, label: [t?.material, null].filter(Boolean).join(" ") || null, allow_swap: true },
        ],
      };
    }
    const tools = Array.from({ length: nTools }, (_, i) => ({ tool: i, slot: next.slots[Math.min(i, next.slots.length - 1)]?.key ?? "" })).filter(
      (t) => t.slot,
    );
    const name = e.name.replace(/\.(gcode|bgcode|3mf)$/i, "").replace(/[_-]+/g, " ");
    edit({ ...next, plates: [...next.plates, { name, file_id: e.id, copies: 1, tools }] });
  };

  const save = async (): Promise<number | null> => {
    setBusy(true);
    setError(null);
    try {
      const v = tplId == null ? await projectsApi.createTemplate(form) : await projectsApi.saveTemplate(tplId, form);
      adopt(v);
      setNote(v.problems.length ? "Saved as a draft" : "All changes saved");
      if (tplId == null) nav(`../templates/${v.id}`, { replace: true });
      return v.id;
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      return null;
    } finally {
      setBusy(false);
    }
  };

  const thumbUrl =
    form.thumb_file_id != null ? files.get(form.thumb_file_id)?.thumb_url ?? null : null;
  const ready = !dirty && tplId != null && problems.length === 0;
  const checks: [boolean | null, string, string][] = [
    [!!form.name.trim(), "Has a name", "#basics"],
    [form.plates.length > 0, `${form.plates.length} plate${form.plates.length === 1 ? "" : "s"} · ${printsPerSet(form)} prints per set`, "#plates"],
    [
      form.plates.every((p) => p.tools.length > 0),
      form.plates.some((p) => !p.tools.length) ? `${form.plates.filter((p) => !p.tools.length).length} plate(s) without a colour slot` : "Every plate uses a slot",
      "#slots",
    ],
    [
      form.slots.every((s) => form.plates.some((p) => p.tools.some((t) => t.slot === s.key))),
      (() => {
        const unused = form.slots.filter((s) => !form.plates.some((p) => p.tools.some((t) => t.slot === s.key)));
        return unused.length ? `Slot ${unused.map((u) => u.key).join(", ")} not used` : "Every slot is used";
      })(),
      "#slots",
    ],
    [form.thumb_file_id != null ? true : null, form.thumb_file_id != null ? "Thumbnail set" : "Thumbnail (optional)", "#thumb"],
  ];
  const done = checks.slice(0, 4).filter((c) => c[0]).length;

  return (
    <div className="prj">
      <div className="prj-head">
        <div className="prj-head__main">
          <Link className="prj-back" to={tplId == null ? "../templates" : ".."} relative="path">
            ← Templates
          </Link>
          <h1 className="prj-title">{form.name.trim() || "New template"}</h1>
        </div>
        <span role="status" className="prj-muted">
          {busy ? "Saving…" : dirty ? "Unsaved changes" : note ?? (tplId != null ? "All changes saved" : "")}
        </span>
        <button type="button" className="prj-btn prj-btn--primary" disabled={busy || (!dirty && tplId != null)} onClick={() => void save()}>
          Save template
        </button>
        {ready ? (
          <Link className="prj-btn" to={`/projects/new?template=${tplId}`}>
            Start a project from it
          </Link>
        ) : (
          <button type="button" className="prj-btn" disabled title={dirty ? "Save first" : "Finish the checklist first"}>
            Start a project from it
          </button>
        )}
      </div>
      {error && <p className="prj-note prj-note--bad" role="alert">{error}</p>}

      <div className="tpl-grid">
        <main style={{ display: "flex", flexDirection: "column", gap: 16, minWidth: 0 }}>
          <section id="basics" aria-label="Basics" className="prj-sec">
            <h2 className="prj-sec__title">Basics</h2>
            <label className="prj-field">
              <span className="prj-field__label">Name</span>
              <input className="prj-input" value={form.name} onChange={(e) => edit({ ...form, name: e.target.value })} placeholder="Chess set" />
            </label>
            <label className="prj-field">
              <span className="prj-field__label">Category</span>
              <input className="prj-input" list="tpl-categories" value={form.category ?? ""} onChange={(e) => edit({ ...form, category: e.target.value || null })} placeholder="Games" />
              <datalist id="tpl-categories">
                {categories.map((c) => (
                  <option key={c} value={c} />
                ))}
              </datalist>
            </label>
            <label className="prj-field">
              <span className="prj-field__label">Notes</span>
              <textarea className="prj-input" value={form.notes ?? ""} onChange={(e) => edit({ ...form, notes: e.target.value || null })} />
            </label>
          </section>

          <section id="plates" aria-label="Plates" className="prj-sec">
            <div className="prj-sec__head">
              <h2 className="prj-sec__title">Plates · {form.plates.length}</h2>
              <span className="prj-muted">{printsPerSet(form)} prints per set</span>
            </div>
            <span className="prj-muted">Files are copied into the template when you save, so deleting them from the Library is safe.</span>
            {form.plates.length === 0 && <p className="prj-muted">Add G-code files from the Library on the right.</p>}
            <ol style={{ listStyle: "none", margin: 0, padding: 0 }}>
              {form.plates.map((p, i) => {
                const f = files.get(p.file_id);
                return (
                  <li key={`${p.file_id}-${i}`} className="tpl-plate">
                    <span className="tpl-plate__n">{i + 1}</span>
                    <Thumb url={f?.thumb_url} />
                    <span style={{ minWidth: 0, display: "flex", flexDirection: "column", gap: 4 }}>
                      <input className="prj-input" aria-label={`Plate ${i + 1} name`} value={p.name} onChange={(e) => setPlate(i, { ...p, name: e.target.value })} style={{ minHeight: 36 }} />
                      <span className={`prj-muted${f?.missing ? " prj-tone--bad" : ""}`}>
                        {f?.file_name ?? `file #${p.file_id}`} · {plateMeta(f)}
                      </span>
                    </span>
                    <span className="prj-step" role="group" aria-label={`Copies of ${p.name} per set`}>
                      <button type="button" aria-label="Fewer copies" disabled={p.copies <= 1} onClick={() => setPlate(i, { ...p, copies: p.copies - 1 })}>
                        −
                      </button>
                      <output>× {p.copies}</output>
                      <button type="button" aria-label="More copies" disabled={p.copies >= MAX_COPIES} onClick={() => setPlate(i, { ...p, copies: p.copies + 1 })}>
                        +
                      </button>
                    </span>
                    <span className="tpl-plate__acts">
                      <button
                        type="button"
                        className="prj-btn prj-btn--small"
                        aria-label={`Move ${p.name} up`}
                        disabled={i === 0}
                        onClick={() => {
                          const a = form.plates.slice();
                          [a[i - 1], a[i]] = [a[i], a[i - 1]];
                          edit({ ...form, plates: a });
                        }}
                      >
                        ↑
                      </button>
                      <button
                        type="button"
                        className="prj-btn prj-btn--small"
                        aria-label={`Remove ${p.name}`}
                        onClick={() =>
                          edit({
                            ...form,
                            plates: form.plates.filter((_, j) => j !== i),
                            thumb_file_id: form.thumb_file_id === p.file_id ? null : form.thumb_file_id,
                          })
                        }
                      >
                        ✕
                      </button>
                    </span>
                  </li>
                );
              })}
            </ol>
          </section>

          <section id="slots" aria-label="Colour slots" className="prj-sec">
            <div className="prj-sec__head">
              <h2 className="prj-sec__title">Colour slots</h2>
              <button
                type="button"
                className="prj-btn prj-btn--small"
                onClick={() => edit({ ...form, slots: [...form.slots, { key: nextSlotKey(form), hex: null, label: null, allow_swap: true }] })}
              >
                + Slot
              </button>
            </div>
            <p className="prj-muted" style={{ margin: 0 }}>
              Tick which slots each plate uses (multi-tool plates: per tool). Preferred colour is only a suggestion when someone starts a project; untick
              “swap” to keep a slot to that colour.
            </p>
            <div className="tpl-matrix-wrap">
              <table className="tpl-matrix">
                <thead>
                  <tr>
                    <th>Slot</th>
                    <th>Preferred</th>
                    <th>Swap</th>
                    {form.plates.map((p, i) => (
                      <th key={i} title={p.name}>
                        P{i + 1}
                      </th>
                    ))}
                    <th />
                  </tr>
                </thead>
                <tbody>
                  {form.slots.map((s, si) => (
                    <tr key={si}>
                      <td>
                        <input
                          className="prj-input"
                          aria-label="Slot name"
                          defaultValue={s.key}
                          key={s.key}
                          onBlur={(e) => {
                            const k = slotKey(e.target.value);
                            if (k && k !== s.key) edit(renameSlot(form, s.key, k));
                            else e.target.value = s.key;
                          }}
                        />
                      </td>
                      <td>
                        <span className="tpl-pref">
                          <input
                            type="color"
                            aria-label={`Preferred colour for ${s.key}`}
                            value={`#${s.hex ?? "8a8f94"}`}
                            onChange={(e) => edit({ ...form, slots: form.slots.map((x, j) => (j === si ? { ...x, hex: e.target.value.slice(1).toUpperCase() } : x)) })}
                          />
                          <input
                            className="prj-input"
                            aria-label={`Preferred filament for ${s.key}`}
                            placeholder="PLA Teal"
                            value={s.label ?? ""}
                            onChange={(e) => edit({ ...form, slots: form.slots.map((x, j) => (j === si ? { ...x, label: e.target.value || null } : x)) })}
                          />
                        </span>
                      </td>
                      <td>
                        <input
                          type="checkbox"
                          aria-label={`Any colour may feed ${s.key}`}
                          checked={s.allow_swap}
                          onChange={(e) => edit({ ...form, slots: form.slots.map((x, j) => (j === si ? { ...x, allow_swap: e.target.checked } : x)) })}
                        />
                      </td>
                      {form.plates.map((p, pi) => {
                        const n = Math.max(1, files.get(p.file_id)?.tools.length ?? 1);
                        return (
                          <td key={pi}>
                            {Array.from({ length: n }, (_, tool) => (
                              <label key={tool} style={{ display: "inline-flex", alignItems: "center", gap: 2, marginRight: 4 }}>
                                <input
                                  type="checkbox"
                                  checked={slotOf(p, tool) === s.key}
                                  aria-label={`${s.key} used on ${p.name}${n > 1 ? ` T${tool}` : ""}`}
                                  onChange={(e) => setPlate(pi, setPlateSlot(p, tool, e.target.checked ? s.key : null))}
                                />
                                {n > 1 && <span className="prj-mono" style={{ fontSize: 11 }}>T{tool}</span>}
                              </label>
                            ))}
                          </td>
                        );
                      })}
                      <td>
                        <button
                          type="button"
                          className="prj-btn prj-btn--small"
                          aria-label={`Remove slot ${s.key}`}
                          onClick={() =>
                            edit({
                              ...form,
                              slots: form.slots.filter((_, j) => j !== si),
                              plates: form.plates.map((p) => ({ ...p, tools: p.tools.filter((t) => t.slot !== s.key) })),
                            })
                          }
                        >
                          ✕
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            {!dirty && problems.length > 0 && <p className="prj-note prj-note--warn">⚠ {problems.join(" ")}</p>}
          </section>

          <section id="thumb" aria-label="Thumbnail" className="prj-sec">
            <div className="prj-sec__head">
              <h2 className="prj-sec__title">
                Thumbnail <span className="prj-muted">optional</span>
              </h2>
            </div>
            <Thumb url={thumbUrl} big alt={form.name ? `${form.name} thumbnail` : ""} />
            <div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
              {form.plates.map((p, i) => (
                <button
                  key={i}
                  type="button"
                  className="prj-btn prj-btn--small"
                  aria-pressed={form.thumb_file_id === p.file_id}
                  disabled={!files.get(p.file_id)?.thumb_url}
                  onClick={() => edit({ ...form, thumb_file_id: p.file_id })}
                >
                  Use plate {i + 1} thumbnail
                </button>
              ))}
              <button type="button" className="prj-btn prj-btn--small" disabled title="Photo upload comes later">
                Upload photo…
              </button>
            </div>
          </section>
        </main>

        <aside style={{ display: "flex", flexDirection: "column", gap: 16, minWidth: 0 }}>
          <section aria-label="Checklist" className="prj-sec">
            <h2 className="prj-sec__title">Ready to use? {done} of 4</h2>
            {checks.map(([ok, label, href], i) => (
              <a key={i} href={href} className="tpl-check">
                <span className={ok ? "prj-tone--ok" : ok === null ? "prj-tone--off" : "prj-tone--warn"}>{ok ? "✓" : ok === null ? "○" : "⚠"}</span>
                {label}
              </a>
            ))}
          </section>
          <LibraryPicker added={new Set(form.plates.map((p) => p.file_id))} onAdd={addPlate} />
        </aside>
      </div>
    </div>
  );
}

function LibraryPicker({ added, onAdd }: { added: Set<number>; onAdd: (e: LibraryEntry) => void }) {
  const [path, setPath] = useState("");
  const [rows, setRows] = useState<LibraryEntry[]>([]);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    lib.list(path).then(
      (r) => {
        setRows(r.entries);
        setError(null);
      },
      (e) => setError(e instanceof Error ? e.message : String(e)),
    );
  }, [path]);
  const shown = useMemo(
    () => rows.filter((e) => e.kind === "folder" || ["gcode", "bgcode", "3mf"].includes(e.format ?? "")),
    [rows],
  );
  const up = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
  return (
    <section aria-label="Add plates" className="prj-sec">
      <h2 className="prj-sec__title">Library · {path || "root"}</h2>
      {error && <p className="prj-note prj-note--bad">{error}</p>}
      <div className="tpl-lib">
        {path && (
          <button type="button" className="prj-btn prj-btn--small" onClick={() => setPath(up)}>
            ↑ Up
          </button>
        )}
        {shown.map((e) =>
          e.kind === "folder" ? (
            <div key={e.id} className="tpl-lib__row">
              <span>📁 {e.name}</span>
              <button type="button" className="prj-btn prj-btn--small" onClick={() => setPath(e.path)}>
                Open
              </button>
            </div>
          ) : (
            <div key={e.id} className="tpl-lib__row">
              <span>
                {e.name}
                <br />
                <span className="prj-muted">{hours(e.print_time_secs)} · {Math.round(e.tools.reduce((a, t) => a + (t.grams ?? 0), 0))} g{e.tools.length > 1 ? ` · ${e.tools.length} tools` : ""}</span>
              </span>
              <button type="button" className="prj-btn prj-btn--small" disabled={added.has(e.id)} onClick={() => onAdd(e)}>
                {added.has(e.id) ? "Added" : "Add"}
              </button>
            </div>
          ),
        )}
        {shown.length === 0 && <span className="prj-muted">No printable files here.</span>}
      </div>
      <span className="prj-muted">Press Add to make a file a plate.</span>
    </section>
  );
}
