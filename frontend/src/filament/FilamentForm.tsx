/**
 * Filament form (task 4.2). Reference: docs/ui-ref/Filament.dc.html lines 261–398.
 *
 * Sections fold up as they are filled ("Next" closes one and opens the next;
 * a folded section shows a one-line summary). Dropdowns are inline. No weight
 * field here (weights belong to spools). TD1 lives inside the colour box.
 */
import { useMemo, useState, type ReactNode } from "react";
import {
  abrasiveReasons,
  COLOUR_KINDS,
  COLOUR_RANGE,
  colourHelp,
  draftFrom as draftFromRow,
  draftToForm,
  emptyDraft,
  filamentApi,
  fitColours,
  materialFull,
  normHex,
  toggleModifier,
  type FilamentDraft,
  type FilamentRow,
  type MaterialCatalogue,
  type VendorRow,
} from "../api/filaments";
import { ColourSwatch } from "./SpoolSwatch";

type SecId = "fil" | "col" | "prt" | "ext";
const ORDER: SecId[] = ["fil", "col", "prt", "ext"];
const TITLE: Record<SecId, string> = { fil: "Filament", col: "Colour", prt: "Print settings", ext: "Extras · optional" };

// ── One collapsing section ────────────────────────────────────────────────

function Section({
  n,
  title,
  open,
  done,
  ok,
  summary,
  onToggle,
  children,
}: {
  n: number;
  title: string;
  open: boolean;
  done: boolean;
  ok: boolean;
  summary: string;
  onToggle: () => void;
  children: ReactNode;
}) {
  const mark = done ? (ok ? "✓" : "!") : String(n);
  const markCls = done ? (ok ? "ff-sec__mark--ok" : "ff-sec__mark--warn") : "";
  return (
    <section aria-label={title} className={`ff-sec${open ? " ff-sec--open" : ""}`}>
      <button type="button" className="ff-sec__head" aria-expanded={open} onClick={onToggle}>
        <span aria-hidden="true" className={`ff-sec__mark ${markCls}`}>
          {mark}
        </span>
        <span className="ff-sec__title">
          <span>{title}</span>
          {!open && <span className="ff-sec__sum">{summary}</span>}
        </span>
        <span aria-hidden="true">{open ? "▴" : "▾"}</span>
      </button>
      {open && <div className="ff-sec__body">{children}</div>}
    </section>
  );
}

// ── The sections, shared by the filament panel and the new spool flow ─────

export interface SectionsProps {
  draft: FilamentDraft;
  setDraft: (d: FilamentDraft) => void;
  catalogue: MaterialCatalogue | null;
  vendors: VendorRow[];
  filaments: FilamentRow[];
  /** Editing this filament (no duplicate warning against itself). */
  editId?: number | null;
  /** Open an existing filament instead (duplicate warning). */
  onOpenDuplicate?: (f: FilamentRow) => void;
  /** Start with every section folded (editing an existing filament). */
  folded?: boolean;
}

export function FilamentSections({
  draft: d,
  setDraft,
  catalogue,
  vendors,
  filaments,
  editId = null,
  onOpenDuplicate,
  folded = false,
}: SectionsProps) {
  const [open, setOpen] = useState<SecId | null>(folded ? null : "fil");
  const [done, setDone] = useState<Partial<Record<SecId, boolean>>>(
    folded ? { fil: true, col: true, prt: true, ext: true } : {},
  );
  const set = (patch: Partial<FilamentDraft>) => setDraft({ ...d, ...patch });
  const reasons = useMemo(() => abrasiveReasons(catalogue), [catalogue]);
  const mats = catalogue?.materials ?? [];
  const matNames = mats.map((m) => m.name);
  if (d.material && !matNames.includes(d.material)) matNames.push(d.material);
  const info = mats.find((m) => m.name === d.material);

  const vq = d.vendor.trim().toLowerCase();
  const vExact = vendors.find((v) => v.name.toLowerCase() === vq);
  const vChips = vendors.filter((v) => !vq || v.name.toLowerCase().startsWith(vq) || v === vExact).slice(0, 6);
  const dup = filaments.find(
    (f) =>
      f.id !== editId &&
      f.name.trim().toLowerCase() === d.name.trim().toLowerCase() &&
      (f.vendor ?? "").toLowerCase() === vq &&
      d.name.trim() !== "",
  );
  const abrHits = d.mods.filter((m) => reasons.has(m)).map((m) => reasons.get(m));
  const [lo, hi] = COLOUR_RANGE[d.kind];

  const complete: Record<SecId, boolean> = {
    fil: !!(d.vendor.trim() && d.name.trim() && d.material),
    col: d.colours.length >= lo && d.colours.every((c) => normHex(c)),
    prt: !!(d.diameter && d.density && d.nozzle && d.bed),
    ext: true,
  };
  const kindLabel = COLOUR_KINDS.find((k) => k[0] === d.kind)?.[1] ?? "";
  const summary: Record<SecId, string> = {
    fil:
      [d.vendor.trim() || "no vendor", d.name.trim() || "no name", materialFull(d.material, d.mods)].join(" · ") +
      (d.abrasive ? " · abrasive" : ""),
    col: `${kindLabel} ${d.colours.join(" ")}${d.kind === "single" && d.td ? ` · TD ${d.td}` : ""}`,
    prt: `${d.diameter} mm · ${d.density} g/cm³ · ${d.nozzle || "—"} / ${d.bed || "—"} °C`,
    ext: [d.price, d.url ? "URL" : "", d.comment ? "comment" : ""].filter(Boolean).join(" · ") || "nothing set",
  };
  const toggle = (k: SecId) => {
    if (open === k) {
      setDone({ ...done, [k]: true });
      setOpen(null);
    } else setOpen(k);
  };
  const next = (k: SecId) => {
    const after = ORDER.slice(ORDER.indexOf(k) + 1).find((x) => !done[x]) ?? null;
    setDone({ ...done, [k]: true });
    setOpen(after);
  };
  const nextLabel = (k: SecId) => {
    const after = ORDER.slice(ORDER.indexOf(k) + 1).find((x) => !done[x]);
    return after ? `Next · ${TITLE[after].replace(" · optional", "")}` : "Done";
  };
  const nextBtn = (k: SecId) => (
    <div className="ff-next">
      <button type="button" className="fil-btn fil-btn--small" disabled={!complete[k]} onClick={() => next(k)}>
        {nextLabel(k)}
      </button>
    </div>
  );

  const setColour = (i: number, v: string) => {
    const h = normHex(v);
    if (!h) return;
    const c = d.colours.slice();
    c[i] = h;
    set({ colours: c });
  };

  return (
    <div className="ff-secs">
      <Section n={1} title={TITLE.fil} open={open === "fil"} done={!!done.fil} ok={complete.fil} summary={summary.fil} onToggle={() => toggle("fil")}>
        <label className="fil-field">
          <span className="fil-field__label">Vendor</span>
          <input
            className="fil-input"
            value={d.vendor}
            onChange={(e) => set({ vendor: e.target.value })}
            placeholder="Type to find or create…"
          />
        </label>
        <div className="ff-chips">
          {vChips.map((v) => (
            <button key={v.id} type="button" className="fil-seg ff-chip" aria-pressed={v === vExact} onClick={() => set({ vendor: v.name })}>
              {v.name}
            </button>
          ))}
          <span className={`fil-sub${d.vendor.trim() && !vExact ? " ff-new" : ""}`}>
            {!d.vendor.trim()
              ? "Pick one, or type a new name."
              : vExact
                ? "✓ Existing vendor"
                : `+ New vendor “${d.vendor.trim()}” is added with the filament.`}
          </span>
        </div>
        <label className="fil-field">
          <span className="fil-field__label">Name</span>
          <input className="fil-input" value={d.name} onChange={(e) => set({ name: e.target.value })} placeholder="e.g. PETG Galaxy Black" />
        </label>
        <div className="ff-row">
          <label className="fil-field">
            <span className="fil-field__label">Material</span>
            <select
              className="fil-input"
              value={d.material}
              onChange={(e) => {
                const m = mats.find((x) => x.name === e.target.value);
                set(
                  m
                    ? { material: m.name, nozzle: String(m.nozzle_temp), bed: String(m.bed_temp), density: String(m.density) }
                    : { material: e.target.value },
                );
              }}
            >
              {matNames.map((m) => (
                <option key={m} value={m}>
                  {m}
                </option>
              ))}
            </select>
          </label>
          <label className="fil-field">
            <span className="fil-field__label">Article no.</span>
            <input className="fil-input" value={d.article} onChange={(e) => set({ article: e.target.value })} placeholder="optional" />
          </label>
        </div>
        <div className="ff-mods">
          <span className="fil-field__label">
            Modifiers <span className="fil-sub">· pick any</span>
          </span>
          {(catalogue?.modifier_groups ?? []).map((g) => (
            <div key={g.label} className="ff-modgroup" role="group" aria-label={g.label}>
              <span className="ff-modgroup__label">{g.label}</span>
              {g.items.map((m) => (
                <button
                  key={m.name}
                  type="button"
                  className="fil-seg ff-chip"
                  aria-pressed={d.mods.includes(m.name)}
                  onClick={() => set(toggleModifier(d.mods, m.name, d.abrasive, reasons))}
                  title={m.abrasive ? `Abrasive: ${m.why}` : undefined}
                >
                  {m.name}
                  {m.abrasive ? " ⚠" : ""}
                </button>
              ))}
            </div>
          ))}
          <span className="fil-sub">
            Saved as <code>{materialFull(d.material, d.mods)}</code>. Modifiers go in the material_modifier field.
          </span>
        </div>
        <label className={`ff-abr${d.abrasive ? " ff-abr--on" : ""}`}>
          <input type="checkbox" checked={d.abrasive} onChange={() => set({ abrasive: !d.abrasive })} />
          <span>Abrasive · needs a hardened nozzle</span>
          {abrHits.length > 0 && <span className="fil-sub">auto: {abrHits.join(", ")}</span>}
        </label>
        {dup && (
          <div className="fil-note fil-note--info">
            You already have “{[dup.vendor, dup.name].filter(Boolean).join(" ")}”.{" "}
            {onOpenDuplicate && (
              <button type="button" className="fil-btn fil-btn--small" onClick={() => onOpenDuplicate(dup)}>
                Open it
              </button>
            )}{" "}
            instead?
          </div>
        )}
        {nextBtn("fil")}
      </Section>

      <Section n={2} title={TITLE.col} open={open === "col"} done={!!done.col} ok={complete.col} summary={summary.col} onToggle={() => toggle("col")}>
        <div role="radiogroup" aria-label="Colour type" className="fil-segs">
          {COLOUR_KINDS.map(([k, label]) => (
            <button
              key={k}
              type="button"
              role="radio"
              aria-checked={d.kind === k}
              aria-pressed={d.kind === k}
              className="fil-seg"
              onClick={() => set({ kind: k, colours: fitColours(k, d.colours), td: k === "single" ? d.td : "" })}
            >
              {label}
            </button>
          ))}
        </div>
        <span className="fil-sub">{colourHelp(d.kind)}</span>
        <div className="ff-preview">
          <ColourSwatch colours={d.colours} kind={d.kind} size={d.kind === "longitudinal" ? 40 : 56} wide={d.kind === "longitudinal"} />
        </div>
        {d.colours.map((c, i) => (
          <div key={i} className="ff-colour">
            <span className="ff-colour__n">{i + 1}</span>
            <input type="color" value={c} onChange={(e) => setColour(i, e.target.value)} aria-label={`Colour ${i + 1}`} />
            <input
              className="fil-input ff-colour__hex"
              defaultValue={c}
              key={c}
              onBlur={(e) => setColour(i, e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && setColour(i, (e.target as HTMLInputElement).value)}
              aria-label={`Colour ${i + 1} hex`}
            />
            <button type="button" className="fil-btn fil-btn--small" disabled title="The TD1 colorimeter arrives with devices">
              ● TD1
            </button>
            {d.colours.length > lo && (
              <button
                type="button"
                className="fil-close"
                aria-label={`Remove colour ${i + 1}`}
                onClick={() => set({ colours: d.colours.filter((_, j) => j !== i) })}
              >
                ✕
              </button>
            )}
          </div>
        ))}
        {d.colours.length < hi && (
          <button type="button" className="fil-btn fil-btn--small ff-add" onClick={() => set({ colours: [...d.colours, "#ffffff"] })}>
            + Colour
          </button>
        )}
        <div className="ff-row">
          <label className="fil-field">
            <span className="fil-field__label">HueForge TD</span>
            <input
              className="fil-input"
              type="number"
              step="0.1"
              min="0"
              value={d.td}
              disabled={d.kind !== "single"}
              onChange={(e) => set({ td: e.target.value })}
            />
          </label>
        </div>
        <span className="fil-sub">
          {d.kind === "single"
            ? "Saved as hue_td. The TD1 fills it once devices are connected."
            : "Multi-colour: TD left empty (HueForge needs one colour per filament)."}
        </span>
        {nextBtn("col")}
      </Section>

      <Section n={3} title={TITLE.prt} open={open === "prt"} done={!!done.prt} ok={complete.prt} summary={summary.prt} onToggle={() => toggle("prt")}>
        <div className="ff-grid4">
          <label className="fil-field">
            <span className="fil-field__label">Diameter mm</span>
            <input className="fil-input" type="number" step="0.05" value={d.diameter} onChange={(e) => set({ diameter: e.target.value })} />
          </label>
          <label className="fil-field">
            <span className="fil-field__label">Density g/cm³</span>
            <input className="fil-input" type="number" step="0.01" value={d.density} onChange={(e) => set({ density: e.target.value })} />
          </label>
          <label className="fil-field">
            <span className="fil-field__label">Nozzle °C</span>
            <input className="fil-input" type="number" value={d.nozzle} onChange={(e) => set({ nozzle: e.target.value })} />
          </label>
          <label className="fil-field">
            <span className="fil-field__label">Bed °C</span>
            <input className="fil-input" type="number" value={d.bed} onChange={(e) => set({ bed: e.target.value })} />
          </label>
        </div>
        <span className="fil-sub">
          {info?.hint ?? "Temperatures and density fill from the material. Change them if this filament differs."}
        </span>
        {nextBtn("prt")}
      </Section>

      <Section n={4} title={TITLE.ext} open={open === "ext"} done={!!done.ext} ok summary={summary.ext} onToggle={() => toggle("ext")}>
        <label className="fil-field">
          <span className="fil-field__label">Price per spool</span>
          <input className="fil-input" inputMode="decimal" value={d.price} onChange={(e) => set({ price: e.target.value })} />
        </label>
        <label className="fil-field">
          <span className="fil-field__label">Product URL</span>
          <input className="fil-input" type="url" value={d.url} onChange={(e) => set({ url: e.target.value })} placeholder="https://" />
        </label>
        <label className="fil-field">
          <span className="fil-field__label">Comment</span>
          <textarea className="fil-input ff-textarea" rows={2} value={d.comment} onChange={(e) => set({ comment: e.target.value })} />
        </label>
        <span className="fil-sub">Spool weights are entered when you add a spool.</span>
        {nextBtn("ext")}
      </Section>
    </div>
  );
}

/** A draft can be saved: vendor, name, material, valid colours. */
export function draftReady(d: FilamentDraft): boolean {
  const [lo, hi] = COLOUR_RANGE[d.kind];
  return !!(
    d.vendor.trim() &&
    d.name.trim() &&
    d.material &&
    d.colours.length >= lo &&
    d.colours.length <= hi &&
    d.colours.every((c) => normHex(c))
  );
}

// ── The filament panel (inspector) ────────────────────────────────────────

export default function FilamentPanel({
  filament,
  catalogue,
  vendors,
  filaments,
  presetVendor,
  onClose,
  onSaved,
  onOpenDuplicate,
}: {
  /** Edit this one; null = new. */
  filament: FilamentRow | null;
  catalogue: MaterialCatalogue | null;
  vendors: VendorRow[];
  filaments: FilamentRow[];
  /** New filament for this vendor ("Save + add filament" from the vendor form). */
  presetVendor?: string;
  onClose: () => void;
  /** `addSpool` = "Save + add spool of it". */
  onSaved: (f: FilamentRow, addSpool: boolean) => void;
  onOpenDuplicate: (f: FilamentRow) => void;
}) {
  const [draft, setDraft] = useState<FilamentDraft>(() =>
    filament ? draftFromRow(filament) : { ...emptyDraft(catalogue?.materials[0]), vendor: presetVendor ?? "" },
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const isNew = !filament;
  const vExact = vendors.some((v) => v.name.toLowerCase() === draft.vendor.trim().toLowerCase());
  const ready = draftReady(draft);

  const save = async (addSpool: boolean) => {
    setBusy(true);
    setError(null);
    try {
      const body = draftToForm(draft, vendors);
      const f = filament ? await filamentApi.updateFilament(filament.id, body) : await filamentApi.createFilament(body);
      onSaved(f, addSpool);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  };

  const title = filament
    ? [filament.vendor, filament.name].filter(Boolean).join(" ")
    : draft.name.trim()
      ? [draft.vendor.trim(), draft.name.trim()].filter(Boolean).join(" ")
      : "New filament";
  return (
    <div className="spool-insp">
      <div className="spool-insp__head">
        <ColourSwatch colours={draft.colours} kind={draft.kind} size={44} />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div className="fil-eyebrow">{isNew ? "New filament" : "Edit filament"}</div>
          <h2 className="spool-insp__name">{title}</h2>
        </div>
        <button type="button" className="fil-close" aria-label="Close" onClick={onClose}>
          ✕
        </button>
      </div>
      {isNew && (
        <p className="fil-note fil-note--info">
          Start from the SpoolmanDB catalogue isn't available yet; fill it in by hand.
        </p>
      )}
      <FilamentSections
        key={filament?.id ?? "new"}
        draft={draft}
        setDraft={setDraft}
        catalogue={catalogue}
        vendors={vendors}
        filaments={filaments}
        editId={filament?.id ?? null}
        onOpenDuplicate={onOpenDuplicate}
        folded={!isNew}
      />
      {error && (
        <p className="fil-note fil-note--bad" role="alert">
          {error}
        </p>
      )}
      <div style={{ flex: 1 }} />
      <span className="fil-sub">
        {filament
          ? `Changes apply to the filament, so every spool of it (${filament.spool_count}) updates.`
          : !ready
            ? "Needs a vendor, a name and its colours."
            : `Creates filament “${title}”${vExact ? "" : ` and vendor “${draft.vendor.trim()}”`} in Spoolman. No spools yet.`}
      </span>
      <div className="load-panel__foot ff-foot">
        <button type="button" className="fil-btn fil-btn--primary" disabled={!ready || busy} onClick={() => void save(false)}>
          {busy ? "Saving…" : filament ? "Save changes" : "Save filament"}
        </button>
        {isNew && (
          <button type="button" className="fil-btn" disabled={!ready || busy} onClick={() => void save(true)}>
            Save + add spool of it
          </button>
        )}
        <button type="button" className="fil-btn" onClick={onClose}>
          Cancel
        </button>
      </div>
    </div>
  );
}

