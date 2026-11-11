/**
 * Printer page → Spools tab (task 4.1): one card per tool / AMS slot; picking a
 * writable slot opens the load panel beside it.
 * Reference: docs/ui-ref/Printer.dc.html lines 238–250 (slot cards).
 */
import { useCallback, useEffect, useState } from "react";
import { useEventStream } from "../hooks/useEventStream";
import { grams, materialLabel, SPOOL_TOPICS, spools, type LoadedSlot } from "../api/spools";
import LoadSpoolPanel from "./LoadSpoolPanel";
import SpoolSwatch from "./SpoolSwatch";
import "./filament.css";

export default function PrinterSpoolsTab({ printerId }: { printerId: number }) {
  const [slots, setSlots] = useState<LoadedSlot[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState<number | null>(null);

  const load = useCallback(async () => {
    try {
      setSlots(await spools.slots(printerId));
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [printerId]);
  useEffect(() => void load(), [load]);
  const onEvent = useCallback(
    (ev: { type: string; [k: string]: unknown }) => {
      if (ev.printer_id === printerId) void load();
    },
    [load, printerId],
  );
  useEventStream(onEvent, SPOOL_TOPICS);

  const anyWritable = (slots ?? []).some((s) => s.writable);

  return (
    <div className="pspools" role="tabpanel" aria-label="Spools">
      <div className="pspools__main">
        <p className="fil-muted" style={{ fontSize: "var(--fw-font-size)" }}>
          {anyWritable
            ? "Pick a slot to load or change its spool. Material, nozzle and remaining filament are checked in the panel."
            : slots?.[0]?.read_only_reason ?? "This printer reports no filament slots."}
        </p>
        {error && <p className="fil-note fil-note--bad" role="alert">{error}</p>}
        <div className="pspools__grid">
          {(slots ?? []).map((s) => {
            const sp = s.spool;
            const pct = sp?.remaining_weight != null && sp.initial_weight
              ? Math.round((sp.remaining_weight / sp.initial_weight) * 100)
              : s.tray?.remaining ?? 0;
            const label = sp
              ? `#${sp.id} ${sp.name}`
              : s.spool_id != null
                ? `Spool #${s.spool_id}`
                : s.tray
                  ? s.tray.material ?? "Unknown"
                  : "Empty";
            const sub = sp
              ? `${grams(sp.remaining_weight)} of ${grams(sp.initial_weight)} left · ${materialLabel(sp)}`
              : s.tray
                ? `Reported by the printer${s.tray.remaining != null ? ` · ${s.tray.remaining}% left` : ""}`
                : s.writable
                  ? "Tap to load a spool"
                  : "Nothing reported";
            return (
              <button
                key={s.tool}
                type="button"
                className="pslot"
                aria-pressed={open === s.tool}
                onClick={() => setOpen(s.tool)}
              >
                <span className="pslot__slot">{s.label}</span>
                <span className="pslot__row">
                  <SpoolSwatch spool={sp} hex={s.tray?.color} size={28} />
                  <b>{label}</b>
                </span>
                <span className="pslot__sub">{sub}</span>
                <span className="fil-bar" aria-hidden="true" style={{ height: 6, width: "100%" }}>
                  <span className="fil-bar__fill" style={{ display: "block", width: `${pct}%` }} />
                </span>
              </button>
            );
          })}
        </div>
      </div>
      {open != null && (
        <div className="pspools__panel">
          <LoadSpoolPanel
            key={open}
            printerId={printerId}
            tool={open}
            onClose={() => setOpen(null)}
            onLoaded={() => void load()}
          />
        </div>
      )}
    </div>
  );
}
