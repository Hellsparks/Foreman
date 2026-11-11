/**
 * "Add plates from Library" dialog (task 5.2). Reference: docs/ui-ref/TemplateEditor.dc.html
 * plate picker: folder breadcrumb, search, a grid of file cards, multi-select.
 * Used by the template editor and by New project → From Library files.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import { lib, type LibraryEntry } from "../api/library";
import { hours, kindLabel } from "../api/projects";
import Thumb from "./Thumb";

const PRINTABLE = ["gcode", "bgcode", "3mf"];

export default function LibraryPicker({
  title = "Add plates from Library",
  verb = "plate",
  taken,
  takenLabel = "Already a plate",
  onAdd,
  onClose,
}: {
  title?: string;
  verb?: string;
  /** Files already in use (shown ticked, not selectable). */
  taken: Set<number>;
  takenLabel?: string;
  onAdd: (files: LibraryEntry[]) => void;
  onClose: () => void;
}) {
  const [path, setPath] = useState("");
  const [rows, setRows] = useState<LibraryEntry[] | null>(null);
  const [q, setQ] = useState("");
  const [sel, setSel] = useState<Map<number, LibraryEntry>>(new Map());
  const [error, setError] = useState<string | null>(null);
  const box = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setRows(null);
    lib.list(path).then(
      (r) => {
        setRows(r.entries);
        setError(null);
      },
      (e) => setError(e instanceof Error ? e.message : String(e)),
    );
  }, [path]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    box.current?.querySelector<HTMLElement>("input")?.focus();
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const shown = useMemo(() => {
    const words = q.toLowerCase().split(/\s+/).filter(Boolean);
    return (rows ?? []).filter(
      (e) =>
        (e.kind === "folder" || PRINTABLE.includes(e.format ?? "")) && words.every((w) => e.name.toLowerCase().includes(w)),
    );
  }, [rows, q]);
  const crumbs = path ? path.split("/") : [];
  const n = sel.size;

  return (
    <div className="lp-scrim" role="presentation" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div ref={box} role="dialog" aria-modal="true" aria-label={title} className="lp">
        <div className="lp__head">
          <b className="lp__title">{title}</b>
          <nav aria-label="Breadcrumb" className="lp__crumbs">
            <button type="button" className="lp__crumb" onClick={() => setPath("")}>
              Library
            </button>
            {crumbs.map((c, i) => (
              <span key={i}>
                {" / "}
                <button type="button" className="lp__crumb" onClick={() => setPath(crumbs.slice(0, i + 1).join("/"))}>
                  {c}
                </button>
              </span>
            ))}
          </nav>
          <input type="search" className="prj-input lp__search" placeholder="Search this folder" aria-label="Search this folder" value={q} onChange={(e) => setQ(e.target.value)} />
          <button type="button" className="prj-btn prj-btn--small" aria-label="Close" onClick={onClose}>
            ✕
          </button>
        </div>
        <div className="lp__grid">
          {error && <p className="prj-note prj-note--bad">{error}</p>}
          {rows == null && !error && <p className="prj-muted">Loading…</p>}
          {shown.map((e) => {
            if (e.kind === "folder")
              return (
                <button key={e.id} type="button" className="lp-card lp-card--folder" onClick={() => setPath(e.path)}>
                  <span className="lp-card__folder" aria-hidden="true">
                    📁
                  </span>
                  <b className="lp-card__name">{e.name}</b>
                  <span className="prj-muted">Folder</span>
                </button>
              );
            const isTaken = taken.has(e.id);
            const on = !isTaken && sel.has(e.id);
            const g = e.tools.reduce((a, t) => a + (t.grams ?? 0), 0);
            const nSlots = e.slots.length || e.tools.length;
            return (
              <button
                key={e.id}
                type="button"
                role="checkbox"
                aria-checked={on || isTaken}
                aria-disabled={isTaken}
                className={`lp-card${on ? " lp-card--on" : ""}`}
                onClick={() => {
                  if (isTaken) return;
                  const m = new Map(sel);
                  if (m.has(e.id)) m.delete(e.id);
                  else m.set(e.id, e);
                  setSel(m);
                }}
              >
                <span className="lp-card__img">
                  <Thumb url={e.thumb_url} />
                  <span className={`lp-card__tick${on ? " lp-card__tick--on" : isTaken ? " lp-card__tick--taken" : ""}`} aria-hidden="true">
                    {on || isTaken ? "✓" : ""}
                  </span>
                </span>
                <b className="lp-card__name" title={e.name}>
                  {e.name}
                </b>
                <span className="prj-muted">
                  {[hours(e.print_time_secs), g ? `${Math.round(g)} g` : null].filter(Boolean).join(" · ")}
                </span>
                <span className={`prj-mono lp-card__kind${e.slot_kind === "change" ? " prj-tone--warn" : ""}`}>
                  {isTaken ? takenLabel : on ? "Selected" : kindLabel(e.slot_kind, nSlots)}
                </span>
              </button>
            );
          })}
          {rows != null && shown.length === 0 && <p className="prj-muted lp__empty">{q ? `Nothing matches “${q}”.` : "No printable files here."}</p>}
        </div>
        <div className="lp__foot">
          <span className="prj-muted" style={{ flex: 1 }}>
            {n ? `${n} selected` : "Pick one or more files"}
          </span>
          <button type="button" className="prj-btn" onClick={onClose}>
            Cancel
          </button>
          <button type="button" className="prj-btn prj-btn--primary" disabled={!n} onClick={() => onAdd([...sel.values()])}>
            {n ? `Add ${n} ${verb}${n === 1 ? "" : "s"}` : `Add ${verb}s`}
          </button>
        </div>
      </div>
    </div>
  );
}
