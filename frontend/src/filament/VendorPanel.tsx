/** Vendor form (task 4.2). Reference: docs/ui-ref/Filament.dc.html lines 399–420. */
import { useState } from "react";
import { filamentApi, parseNumber, type VendorRow } from "../api/filaments";
import { ExternalLink, safeHref } from "../ui";

export default function VendorPanel({
  vendor,
  vendors,
  onClose,
  onSaved,
}: {
  /** Edit this one; null = new. */
  vendor: VendorRow | null;
  vendors: VendorRow[];
  onClose: () => void;
  /** `addFilament` = "Save + add filament". */
  onSaved: (v: VendorRow, addFilament: boolean) => void;
}) {
  const [name, setName] = useState(vendor?.name ?? "");
  const [empty, setEmpty] = useState(vendor?.empty_spool_weight != null ? String(vendor.empty_spool_weight) : "");
  const [url, setUrl] = useState(vendor?.website ?? "");
  const [comment, setComment] = useState(vendor?.comment ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const dup = vendors.some((v) => v.id !== vendor?.id && v.name.toLowerCase() === name.trim().toLowerCase());
  const emptyNum = parseNumber(empty);
  const emptyBad = empty.trim() !== "" && (emptyNum == null || emptyNum < 0);
  const ready = name.trim() !== "" && !dup && !emptyBad;

  const save = async (addFilament: boolean) => {
    setBusy(true);
    setError(null);
    try {
      const body = { name: name.trim(), empty_spool_weight: emptyNum, website: url.trim() || null, comment: comment.trim() || null };
      const v = vendor ? await filamentApi.updateVendor(vendor.id, body) : await filamentApi.createVendor(body);
      onSaved(v, addFilament);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  };

  return (
    <div className="spool-insp">
      <div className="spool-insp__head">
        <div style={{ flex: 1, minWidth: 0 }}>
          <div className="fil-eyebrow">{vendor ? "Edit vendor" : "New vendor"}</div>
          <h2 className="spool-insp__name">{vendor ? vendor.name : name.trim() || "New vendor"}</h2>
        </div>
        <button type="button" className="fil-close" aria-label="Close" onClick={onClose}>
          ✕
        </button>
      </div>
      <section className="spool-insp__box ff-vendor">
        <label className="fil-field">
          <span className="fil-field__label">Name</span>
          <input className="fil-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. Sunlu" />
        </label>
        {dup && <p className="fil-note fil-note--bad">“{name.trim()}” already exists.</p>}
        <label className="fil-field">
          <span className="fil-field__label">Empty spool weight g</span>
          <input
            className="fil-input"
            type="number"
            min="0"
            value={empty}
            onChange={(e) => setEmpty(e.target.value)}
            placeholder="e.g. 200"
            aria-invalid={emptyBad}
          />
          <span className="fil-sub">
            Default for this vendor's spools. The scale uses it to work out how much filament is left. Weigh an empty one to get it exact.
          </span>
        </label>
        <label className="fil-field">
          <span className="fil-field__label">Website / store</span>
          <input className="fil-input" type="url" value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://" />
          {safeHref(url) && <ExternalLink href={url} label="Open vendor website" />}
          <span className="fil-sub">Used for the shopping list's store link when a filament has no link of its own.</span>
        </label>
        <label className="fil-field">
          <span className="fil-field__label">Comment</span>
          <textarea className="fil-input ff-textarea" rows={2} value={comment} onChange={(e) => setComment(e.target.value)} />
        </label>
      </section>
      {vendor && (
        <p className="fil-muted">
          {vendor.filament_count} filament{vendor.filament_count === 1 ? "" : "s"} from {vendor.name}.
        </p>
      )}
      {error && (
        <p className="fil-note fil-note--bad" role="alert">
          {error}
        </p>
      )}
      <div style={{ flex: 1 }} />
      <div className="load-panel__foot ff-foot">
        <button type="button" className="fil-btn fil-btn--primary" disabled={!ready || busy} onClick={() => void save(false)}>
          {busy ? "Saving…" : vendor ? "Save changes" : "Save vendor"}
        </button>
        {!vendor && (
          <button type="button" className="fil-btn" disabled={!ready || busy} onClick={() => void save(true)}>
            Save + add filament
          </button>
        )}
        <button type="button" className="fil-btn" onClick={onClose}>
          Cancel
        </button>
      </div>
    </div>
  );
}
