/** Locations tab (task 4.3): storage locations, spool counts, rename. */
import { useState } from "react";
import { inventoryApi, kg, type Locations } from "../api/inventory";

/** Spools-tab location filter value for "no location". */
export const NO_LOCATION = "\u0000none";

export default function LocationsTab({
  data,
  onShow,
  onRenamed,
}: {
  data: Locations | null;
  /** Show the Spools tab filtered to this location (or NO_LOCATION). */
  onShow: (loc: string) => void;
  onRenamed: (msg: string) => void;
}) {
  const [editing, setEditing] = useState<string | null>(null);
  const [to, setTo] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!data) return <p className="fil-placeholder">Loading locations…</p>;
  const existing = (n: string) => data.locations.some((l) => l.name === n.trim() && l.name !== editing);

  const save = async (from: string) => {
    setBusy(true);
    setError(null);
    try {
      const r = await inventoryApi.renameLocation(from, to);
      setEditing(null);
      onRenamed(`Renamed ${from} → ${r.name} · ${r.spools_moved} spool${r.spools_moved === 1 ? "" : "s"} moved`);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <div className="fil-count">
        {data.locations.length} location{data.locations.length === 1 ? "" : "s"} · {data.unplaced} spool
        {data.unplaced === 1 ? "" : "s"} without one
      </div>
      {error && (
        <p className="fil-note fil-note--bad" role="alert">
          {error}
        </p>
      )}
      <div className="fil-table-wrap page-fill">
        <table className="fil-table" style={{ minWidth: 560 }}>
          <thead>
            <tr>
              <th>Location</th>
              <th>Spools</th>
              <th>Filament</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {data.locations.map((l) => (
              <tr key={l.name}>
                <td>
                  {editing === l.name ? (
                    <form
                      className="loc-rename"
                      onSubmit={(e) => {
                        e.preventDefault();
                        void save(l.name);
                      }}
                    >
                      <input
                        className="fil-input"
                        aria-label={`New name for ${l.name}`}
                        value={to}
                        maxLength={64}
                        autoFocus
                        onChange={(e) => setTo(e.target.value)}
                      />
                      <button type="submit" className="fil-btn fil-btn--small" disabled={busy || !to.trim() || to.trim() === l.name}>
                        {busy ? "Saving…" : existing(to) ? "Merge" : "Save"}
                      </button>
                      <button type="button" className="fil-btn fil-btn--small" onClick={() => setEditing(null)}>
                        Cancel
                      </button>
                      {existing(to) && (
                        <span className="fil-sub" style={{ flexBasis: "100%" }}>
                          {to.trim()} already exists: its spools and these end up together.
                        </span>
                      )}
                    </form>
                  ) : (
                    <b>{l.name}</b>
                  )}
                </td>
                <td className="fil-mono">{l.spool_count}</td>
                <td className="fil-mono">{l.spool_count ? kg(l.grams) : "—"}</td>
                <td style={{ textAlign: "right", whiteSpace: "nowrap" }}>
                  <button type="button" className="fil-btn fil-btn--small" disabled={!l.spool_count} onClick={() => onShow(l.name)}>
                    Show spools
                  </button>{" "}
                  <button
                    type="button"
                    className="fil-btn fil-btn--small"
                    onClick={() => {
                      setEditing(l.name);
                      setTo(l.name);
                      setError(null);
                    }}
                  >
                    Rename
                  </button>
                </td>
              </tr>
            ))}
            {data.unplaced > 0 && (
              <tr>
                <td className="fil-sub">No location</td>
                <td className="fil-mono">{data.unplaced}</td>
                <td />
                <td style={{ textAlign: "right" }}>
                  <button type="button" className="fil-btn fil-btn--small" onClick={() => onShow(NO_LOCATION)}>
                    Show spools
                  </button>
                </td>
              </tr>
            )}
            {data.locations.length === 0 && data.unplaced === 0 && (
              <tr>
                <td colSpan={4} style={{ padding: "var(--fw-space-4)" }} className="fil-placeholder">
                  No locations yet. Set one on a spool (Edit spool → Location).
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      <p className="fil-sub">Renaming moves every spool on it, archived ones too, and keeps Spoolman's own location list in step.</p>
    </>
  );
}
