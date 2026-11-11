/**
 * Inventory tab (task 4.3). Reference: docs/ui-ref/Filament.dc.html lines 148–195,
 * behaviour in renderVals (stock / status / buy / shopping list).
 */
import { useMemo, useState } from "react";
import {
  inventoryApi,
  kg,
  label,
  money,
  shoppingText,
  sizeLabel,
  stepTarget,
  storeUrl,
  type Inventory,
  type InventoryRow,
  type TrackFilament,
} from "../api/inventory";
import { materialFull, type FilamentRow, type VendorRow } from "../api/filaments";
import { ColourSwatch } from "./SpoolSwatch";
import { ExternalLink } from "../ui";

const STATUS: Record<InventoryRow["status"], [string, string]> = {
  ok: ["OK", "fil-tag--ok"],
  buy: ["BUY", "fil-tag--warn"],
  out: ["OUT", "fil-tag--bad"],
};

export default function InventoryTab({
  inv,
  filaments,
  vendors,
  onChange,
  onBought,
  onToast,
}: {
  inv: Inventory | null;
  filaments: FilamentRow[];
  vendors: VendorRow[];
  onChange: (inv: Inventory) => void;
  /** Open the new-spool flow for `row.buy` spools of this filament. */
  onBought: (row: InventoryRow) => void;
  onToast: (msg: string) => void;
}) {
  const [pickerOpen, setPickerOpen] = useState(false);
  const [pq, setPq] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<number | null>(null);

  const site = useMemo(() => {
    const m = new Map(vendors.map((v) => [v.id, v.website]));
    return (id: number) => m.get(id);
  }, [vendors]);

  const tracked = inv?.tracked ?? [];
  const trackedIds = new Set(tracked.map((r) => r.filament.id));
  const untracked = filaments.filter((f) => !trackedIds.has(f.id));
  const pickHits = untracked.filter((f) => {
    const words = pq.toLowerCase().split(/\s+/).filter(Boolean);
    const hay = [f.vendor, f.name, f.material, ...f.modifiers].filter(Boolean).join(" ").toLowerCase();
    return words.every((w) => hay.includes(w));
  });
  const shop = tracked.filter((r) => r.buy > 0);

  const run = async (id: number, p: Promise<Inventory>) => {
    setBusy(id);
    setError(null);
    try {
      onChange(await p);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };
  const set = (r: InventoryRow, field: keyof TrackFilament, delta: number) => {
    const next = stepTarget({ target: r.target, buy_below: r.buy_below }, field, delta);
    if (next.target === r.target && next.buy_below === r.buy_below) return;
    void run(r.filament.id, inventoryApi.track(r.filament.id, next));
  };
  const track = (f: FilamentRow) => {
    const t = Math.max(1, f.spool_count);
    void run(f.id, inventoryApi.track(f.id, { target: t, buy_below: 1 }));
  };
  const copy = async () => {
    if (!inv) return;
    const text = shoppingText(inv, (r) => storeUrl(r, site));
    try {
      await navigator.clipboard.writeText(text);
      onToast(`Shopping list copied (${shop.length} line${shop.length === 1 ? "" : "s"})`);
    } catch {
      setError("The browser blocked the clipboard. Select the list and copy it by hand.");
    }
  };

  if (!inv) return <p className="fil-placeholder">Loading inventory…</p>;
  const cur = inv.currency;
  const jobs = new Set(inv.reservations.map((r) => r.queue_item_id)).size;
  const tiles: { k: string; v: string; sub?: string; warn?: boolean }[] = [
    { k: "Tracked filaments", v: String(tracked.length), sub: `of ${filaments.length}` },
    { k: "Need restocking", v: String(inv.need_count), warn: inv.need_count > 0 },
    {
      k: "Est. restock cost",
      v: money(inv.est_cost, cur),
      sub: inv.unpriced ? `+${inv.unpriced} without a price` : undefined,
    },
    { k: "On hand", v: kg(inv.grams_on_hand), sub: `${inv.spool_count} spools` },
    {
      k: "Reserved",
      v: kg(inv.grams_reserved),
      sub: jobs ? `by ${jobs} queued job${jobs === 1 ? "" : "s"}` : "nothing queued holds filament",
    },
  ];

  return (
    <div className="inv">
      <div className="inv-tiles">
        {tiles.map((t) => (
          <div key={t.k} className={`inv-tile${t.warn ? " inv-tile--warn" : ""}`}>
            <div className="inv-tile__v">{t.v}</div>
            <div className="inv-tile__k">
              {t.k}
              {t.sub && <span className="fil-sub"> · {t.sub}</span>}
            </div>
          </div>
        ))}
      </div>
      {error && (
        <p className="fil-note fil-note--bad" role="alert">
          {error}
        </p>
      )}

      <section aria-label="Shopping list" className="inv-sec">
        <div className="inv-sec__head">
          <h2 className="inv-sec__title">
            Shopping list · {shop.length} item{shop.length === 1 ? "" : "s"}
          </h2>
          <button type="button" className="fil-btn fil-btn--small" disabled={!shop.length} onClick={() => void copy()}>
            Copy list
          </button>
        </div>
        {shop.length === 0 && (
          <div className="inv-empty">
            {tracked.length
              ? "Nothing to buy. Everything tracked is at or above its buy-below level."
              : "Track a filament below to give it a target; it shows up here when it runs low."}
          </div>
        )}
        {shop.map((r) => {
          const url = storeUrl(r, site);
          return (
            <div key={r.filament.id} className="inv-shop">
              <ColourSwatch colours={r.filament.colours} kind={r.filament.colour_kind} size={28} />
              <span className="inv-shop__name">
                <b>{r.filament.name}</b> <span className="fil-sub">— {sizeLabel(r.filament.weight)}</span>
                <br />
                <span className="fil-sub">
                  {[r.filament.vendor, materialFull(r.filament.material ?? "", r.filament.modifiers)].filter(Boolean).join(" · ")} · have{" "}
                  {r.stock}, target {r.target}
                </span>
              </span>
              <span className="inv-shop__n">× {r.buy}</span>
              <span className="inv-shop__cost">{r.line_cost != null ? money(r.line_cost, cur) : "no price"}</span>
              <span className="inv-shop__acts">
                {url ? (
                  <ExternalLink href={url} label={`Store page for ${label(r)}`} className="fil-btn fil-btn--small">
                    Store
                  </ExternalLink>
                ) : (
                  <span className="fil-sub">No store link</span>
                )}
                <button type="button" className="fil-btn fil-btn--small" onClick={() => onBought(r)}>
                  Bought → add {r.buy}
                </button>
              </span>
            </div>
          );
        })}
        {shop.length > 0 && (
          <div className="inv-total">
            <span className="fil-sub">Estimated total</span>
            <b>{money(inv.est_cost, cur)}</b>
          </div>
        )}
      </section>

      <section aria-label="Tracked filaments" className="inv-sec">
        <div className="inv-sec__head">
          <h2 className="inv-sec__title">Tracked filaments · {tracked.length}</h2>
          <button type="button" className="fil-btn fil-btn--small" aria-expanded={pickerOpen} onClick={() => setPickerOpen(!pickerOpen)}>
            + Track filament
          </button>
        </div>
        {pickerOpen && (
          <div className="inv-picker">
            <input
              type="search"
              className="fil-input"
              aria-label="Find a filament to track"
              placeholder={`Find one of ${untracked.length} untracked filaments…`}
              value={pq}
              onChange={(e) => setPq(e.target.value)}
            />
            <div className="inv-picker__list">
              {pickHits.slice(0, 24).map((f) => (
                <button key={f.id} type="button" className="fil-btn fil-btn--small" disabled={busy === f.id} onClick={() => track(f)}>
                  <ColourSwatch colours={f.colours} kind={f.colour_kind} size={14} />
                  {[f.vendor, f.name].filter(Boolean).join(" ")}
                  <span className="fil-sub">
                    {materialFull(f.material ?? "", f.modifiers)} · {f.spool_count} spool{f.spool_count === 1 ? "" : "s"}
                  </span>
                </button>
              ))}
              {pickHits.length > 24 && <span className="fil-sub">+{pickHits.length - 24} more, narrow the search</span>}
              {untracked.length === 0 && <span className="fil-sub">All filaments are tracked.</span>}
              {untracked.length > 0 && pickHits.length === 0 && <span className="fil-sub">No untracked filament matches.</span>}
            </div>
          </div>
        )}
        <div className="fil-table-wrap inv-table-wrap">
          <table className="fil-table">
            <thead>
              <tr>
                <th>Filament</th>
                <th>Material</th>
                <th>Size</th>
                <th className="inv-c">In stock</th>
                <th className="inv-c">Target</th>
                <th className="inv-c">Buy below</th>
                <th>Status</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {tracked.map((r) => {
                const [st, cls] = STATUS[r.status];
                const dis = busy === r.filament.id;
                return (
                  <tr key={r.filament.id}>
                    <td>
                      <span className="inv-fil">
                        <ColourSwatch colours={r.filament.colours} kind={r.filament.colour_kind} size={24} />
                        <span>
                          <b>{r.filament.name}</b>
                          <br />
                          <span className="fil-sub">{r.filament.vendor ?? "No vendor"}</span>
                        </span>
                      </span>
                    </td>
                    <td className="fil-mono">{materialFull(r.filament.material ?? "—", r.filament.modifiers)}</td>
                    <td>{sizeLabel(r.filament.weight)}</td>
                    <td className="inv-c">
                      <b className={`fil-mono inv-have inv-have--${r.status}`}>{r.stock}</b>
                      <br />
                      <span className="fil-sub">
                        {Math.round(r.grams)} g{r.grams_reserved > 0 ? ` · ${Math.round(r.grams_reserved)} g reserved` : ""}
                      </span>
                    </td>
                    <td className="inv-c">
                      <Stepper
                        value={r.target}
                        what="target"
                        disabled={dis}
                        onDown={() => set(r, "target", -1)}
                        onUp={() => set(r, "target", 1)}
                      />
                    </td>
                    <td className="inv-c">
                      <Stepper
                        value={r.buy_below}
                        what="buy-below"
                        disabled={dis}
                        onDown={() => set(r, "buy_below", -1)}
                        onUp={() => set(r, "buy_below", 1)}
                      />
                    </td>
                    <td>
                      <span className={`fil-tag ${cls}`}>{st}</span>
                    </td>
                    <td style={{ textAlign: "right" }}>
                      <button
                        type="button"
                        className="fil-btn fil-btn--small"
                        disabled={dis}
                        onClick={() => {
                          void run(r.filament.id, inventoryApi.untrack(r.filament.id));
                          onToast(`Stopped tracking ${label(r)}`);
                        }}
                      >
                        Untrack
                      </button>
                    </td>
                  </tr>
                );
              })}
              {tracked.length === 0 && (
                <tr>
                  <td colSpan={8} style={{ padding: 16 }} className="fil-placeholder">
                    No filament is tracked yet. Use + Track filament.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
        <div className="inv-foot fil-sub">
          Target = how many spools you want on hand. Buy below = when it goes on the shopping list. A spool counts as
          stock while more than 50 g is left after what queued jobs have reserved.
        </div>
      </section>

      {inv.reservations.length > 0 && (
        <details className="inv-sec">
          <summary className="inv-sec__title inv-summary">
            Reserved by the queue · {kg(inv.grams_reserved)}
          </summary>
          {inv.reservations.map((r) => (
            <div key={`${r.queue_item_id}-${r.spool_id}`} className="inv-res">
              <span style={{ flex: 1 }}>{r.file_name}</span>
              <span className="fil-mono">spool #{r.spool_id}</span>
              <span className="fil-mono">{Math.round(r.grams)} g</span>
            </div>
          ))}
        </details>
      )}
    </div>
  );
}

function Stepper({
  value,
  what,
  disabled,
  onDown,
  onUp,
}: {
  value: number;
  what: string;
  disabled: boolean;
  onDown: () => void;
  onUp: () => void;
}) {
  return (
    <span className="inv-step">
      <button type="button" aria-label={`Lower ${what}`} disabled={disabled} onClick={onDown}>
        −
      </button>
      <output aria-label={what}>{value}</output>
      <button type="button" aria-label={`Raise ${what}`} disabled={disabled} onClick={onUp}>
        +
      </button>
    </span>
  );
}
