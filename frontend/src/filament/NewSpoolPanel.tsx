/**
 * New spool flow (task 4.2). Reference: docs/ui-ref/Filament.dc.html lines 443–554.
 *
 * Search existing filaments first; create one inline only when nothing matches.
 * Then the spool fields and how many. The SpoolmanDB catalogue step is skipped
 * until the external catalogue exists (spoolman-rs C.3 is a stub).
 */
import { useMemo, useState } from "react";
import {
  draftToForm,
  emptyDraft,
  filamentApi,
  materialFull,
  parseNumber,
  type CreatedSpools,
  type FilamentDraft,
  type FilamentRow,
  type MaterialCatalogue,
  type VendorRow,
} from "../api/filaments";
import { draftReady, FilamentSections } from "./FilamentForm";
import { ColourSwatch } from "./SpoolSwatch";

const MAX_QTY = 50;

function label(f: FilamentRow): string {
  return [f.vendor, f.name].filter(Boolean).join(" ");
}

export default function NewSpoolPanel({
  catalogue,
  vendors,
  filaments,
  locations,
  preset,
  presetQty,
  onClose,
  onCreated,
  onLoad,
}: {
  catalogue: MaterialCatalogue | null;
  vendors: VendorRow[];
  filaments: FilamentRow[];
  locations: string[];
  /** Start with this filament picked ("Save + add spool of it"). */
  preset?: FilamentRow | null;
  /** Start with this many spools (shopping list "Bought → add N"). */
  presetQty?: number;
  onClose: () => void;
  onCreated: (c: CreatedSpools) => void;
  /** "Load onto a printer…" after creating. */
  onLoad: (spoolId: number) => void;
}) {
  const [fq, setFq] = useState(preset ? label(preset) : "");
  const [comboOpen, setComboOpen] = useState(!preset);
  const [picked, setPicked] = useState<FilamentRow | null>(preset ?? null);
  const [creating, setCreating] = useState(false);
  const [draft, setDraft] = useState<FilamentDraft>(() => emptyDraft(catalogue?.materials[0]));
  const [weight, setWeight] = useState(String(preset?.weight ?? 1000));
  const [empty, setEmpty] = useState<string | null>(null); // null = follow the default
  const [price, setPrice] = useState<string | null>(null);
  const [location, setLocation] = useState("");
  const [lot, setLot] = useState("");
  const [qty, setQty] = useState(Math.min(MAX_QTY, Math.max(1, presetQty ?? 1)));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<CreatedSpools | null>(null);

  const hits = useMemo(() => {
    const words = fq.toLowerCase().split(/\s+/).filter(Boolean);
    return filaments
        .filter((f) => {
          const hay = [f.vendor, f.name, f.material, ...f.modifiers, ...f.colours].filter(Boolean).join(" ").toLowerCase();
          return words.every((w) => hay.includes(w.replace(/^#/, "")));
        })
        .slice(0, 6);
  }, [filaments, fq]);
  const exact = filaments.some(
    (f) => f.name.toLowerCase() === fq.trim().toLowerCase() || label(f).toLowerCase() === fq.trim().toLowerCase(),
  );
  const canCreate = fq.trim() !== "" && !exact && !creating;

  // Defaults the spool fields follow until the user changes them.
  const draftVendor = vendors.find((v) => v.name.toLowerCase() === draft.vendor.trim().toLowerCase());
  const defEmpty = creating ? draftVendor?.empty_spool_weight ?? null : picked?.spool_weight ?? null;
  const defEmptyFrom = creating ? draftVendor?.name : picked?.vendor;
  const defPrice = creating ? parseNumber(draft.price) : picked?.price ?? null;
  const emptyVal = empty ?? (defEmpty != null ? String(defEmpty) : "");
  const priceVal = price ?? (defPrice != null ? String(defPrice) : "");

  const pick = (f: FilamentRow) => {
    setPicked(f);
    setCreating(false);
    setFq(label(f));
    setComboOpen(false);
    setEmpty(null);
    setPrice(null);
    if (f.weight != null) setWeight(String(f.weight));
  };
  const startCreate = () => {
    setCreating(true);
    setPicked(null);
    setComboOpen(false);
    setDraft({ ...emptyDraft(catalogue?.materials[0]), name: fq.trim() });
    setEmpty(null);
    setPrice(null);
  };

  const weightNum = parseNumber(weight);
  const filamentOk = creating ? draftReady(draft) : picked != null;
  const ready = filamentOk && weightNum != null && weightNum > 0 && qty >= 1 && qty <= MAX_QTY;

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      const res = await filamentApi.createSpools({
        filament_id: creating ? null : picked?.id ?? null,
        new_filament: creating ? draftToForm(draft, vendors) : null,
        initial_weight: weightNum,
        spool_weight: parseNumber(emptyVal),
        location: location.trim() || null,
        lot_nr: lot.trim() || null,
        price: parseNumber(priceVal),
        comment: null,
        quantity: qty,
      });
      setDone(res);
      onCreated(res);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const reset = () => {
    setDone(null);
    setQty(1);
    setLot("");
  };

  const head = (
    <div className="spool-insp__head">
      <div style={{ flex: 1, minWidth: 0 }}>
        <div className="fil-eyebrow">New spool</div>
        <h2 className="spool-insp__name">
          {done ? label(done.filament) : creating ? draft.name.trim() || "New filament" : picked ? label(picked) : "Add a spool"}
        </h2>
      </div>
      <button type="button" className="fil-close" aria-label="Close" onClick={onClose}>
        ✕
      </button>
    </div>
  );

  if (done) {
    const ids = done.spools.map((s) => `#${s.id}`).join(", ");
    return (
      <div className="spool-insp">
        {head}
        <p className="fil-note fil-note--ok" role="status">
          ✓ Added {done.spools.length} spool{done.spools.length === 1 ? "" : "s"} ({ids}) of {label(done.filament)}
          {done.filament_created ? " — new filament saved" : ""}.
        </p>
        <p className="fil-muted">What next?</p>
        <div className="spool-insp__actions">
          {done.spools[0] && (
            <button type="button" className="fil-btn fil-btn--primary" onClick={() => onLoad(done.spools[0].id)}>
              Load onto a printer…
            </button>
          )}
          <button type="button" className="fil-btn" onClick={reset}>
            Add more of it
          </button>
          <button type="button" className="fil-btn" style={{ gridColumn: "1 / -1" }} onClick={onClose}>
            Done
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="spool-insp">
      {head}
      <div className="ns-combo">
        <label className="fil-field">
          <span className="fil-field__label">Filament</span>
          <input
            className="fil-input"
            role="combobox"
            aria-expanded={comboOpen}
            aria-controls="ns-filament-list"
            value={fq}
            onChange={(e) => {
              setFq(e.target.value);
              setComboOpen(true);
              if (picked) setPicked(null);
            }}
            onFocus={() => setComboOpen(true)}
            placeholder="Search your filaments: vendor, name, material…"
          />
        </label>
        {comboOpen && (
          <div role="listbox" id="ns-filament-list" aria-label="Filaments" className="ns-list">
            {hits.map((f) => (
              <button key={f.id} type="button" role="option" aria-selected={picked?.id === f.id} className="ns-opt" onClick={() => pick(f)}>
                <ColourSwatch colours={f.colours} kind={f.colour_kind} size={20} />
                <span className="ns-opt__name">
                  {label(f)}
                  <span className="fil-sub"> · {materialFull(f.material ?? "", f.modifiers)}</span>
                </span>
                <span className="fil-sub">
                  {f.spool_count} spool{f.spool_count === 1 ? "" : "s"}
                </span>
              </button>
            ))}
            {hits.length === 0 && <span className="fil-sub ns-empty">No filament matches “{fq.trim()}”.</span>}
            {canCreate && (
              <button type="button" role="option" aria-selected={false} className="ns-opt ns-opt--create" onClick={startCreate}>
                + Create “{fq.trim()}”
              </button>
            )}
          </div>
        )}
        <span className="fil-sub">Start from the SpoolmanDB catalogue isn't available yet.</span>
      </div>

      {creating && (
        <div className="ns-new">
          <div className="fil-eyebrow">New filament · saved with the spool</div>
          <FilamentSections draft={draft} setDraft={setDraft} catalogue={catalogue} vendors={vendors} filaments={filaments} onOpenDuplicate={pick} />
        </div>
      )}

      {(picked || creating) && (
        <section aria-label="Spool" className="spool-insp__box ns-spool">
          <div className="ff-row">
            <label className="fil-field">
              <span className="fil-field__label">Net weight g</span>
              <input className="fil-input" type="number" min="1" value={weight} onChange={(e) => setWeight(e.target.value)} />
            </label>
            <label className="fil-field">
              <span className="fil-field__label">Empty spool g</span>
              <input className="fil-input" type="number" min="0" value={emptyVal} onChange={(e) => setEmpty(e.target.value)} placeholder="unknown" />
            </label>
          </div>
          <span className="fil-sub">
            {empty == null && defEmpty != null && defEmptyFrom
              ? `Empty spool weight comes from ${defEmptyFrom} (${defEmpty} g). Change it if this spool differs.`
              : "The scale uses the empty spool weight to work out how much filament is left."}
          </span>
          <div className="ff-row">
            <label className="fil-field">
              <span className="fil-field__label">Price per spool</span>
              <input className="fil-input" inputMode="decimal" value={priceVal} onChange={(e) => setPrice(e.target.value)} />
            </label>
            <label className="fil-field">
              <span className="fil-field__label">Location</span>
              <input className="fil-input" list="ns-locations" value={location} onChange={(e) => setLocation(e.target.value)} placeholder="Shelf A1" />
              <datalist id="ns-locations">
                {locations.map((l) => (
                  <option key={l} value={l} />
                ))}
              </datalist>
            </label>
          </div>
          <label className="fil-field">
            <span className="fil-field__label">Lot number</span>
            <input className="fil-input" value={lot} onChange={(e) => setLot(e.target.value)} placeholder="optional" />
          </label>
          <div className="ns-qty">
            <span className="fil-field__label">How many spools</span>
            <div className="ns-qty__ctl">
              <button type="button" className="fil-btn fil-btn--small" aria-label="Fewer" disabled={qty <= 1} onClick={() => setQty(qty - 1)}>
                −
              </button>
              <output aria-live="polite" className="ns-qty__n">
                {qty}
              </output>
              <button type="button" className="fil-btn fil-btn--small" aria-label="More" disabled={qty >= MAX_QTY} onClick={() => setQty(qty + 1)}>
                +
              </button>
            </div>
          </div>
        </section>
      )}

      {error && (
        <p className="fil-note fil-note--bad" role="alert">
          {error}
        </p>
      )}
      <div style={{ flex: 1 }} />
      <div className="load-panel__foot ff-foot">
        <button type="button" className="fil-btn fil-btn--primary" disabled={!ready || busy} onClick={() => void save()}>
          {busy ? "Saving…" : `Add ${qty} spool${qty === 1 ? "" : "s"}`}
        </button>
        <button type="button" className="fil-btn" onClick={onClose}>
          Cancel
        </button>
      </div>
    </div>
  );
}
