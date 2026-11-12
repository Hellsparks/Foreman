/**
 * Filters behind one "Filter" chip on phone (doc 06 §4), showing the active
 * count; the chip opens the page's own filter controls in a bottom sheet.
 * Desktop: the controls render inline, unchanged.
 */
import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { usePhone } from "../hooks/useMediaQuery";

interface Props {
  /** How many filters differ from their default. */
  active: number;
  /** Shown beside the chip on phone (e.g. a "⋯" menu, a result count). */
  extra?: ReactNode;
  children: ReactNode;
}

export default function FilterSheet({ active, extra, children }: Props) {
  const phone = usePhone();
  const [open, setOpen] = useState(false);
  const id = useId();
  const chip = useRef<HTMLButtonElement>(null);
  const close = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!open) return;
    close.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
        chip.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  if (!phone) return <>{children}</>;
  return (
    <>
      <div className="ui-filterbar">
        <button
          ref={chip}
          type="button"
          className="ui-filter-chip"
          aria-haspopup="dialog"
          aria-expanded={open}
          aria-controls={id}
          data-active={active > 0 ? "true" : undefined}
          onClick={() => setOpen(true)}
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden="true">
            <path d="M4 6h16M7 12h10M10 18h4" />
          </svg>
          Filter{active > 0 ? ` · ${active}` : ""}
        </button>
        {extra}
      </div>
      {open && <div className="phone-sheet__scrim" onClick={() => setOpen(false)} />}
      <div id={id} className="phone-sheet" role="dialog" aria-label="Filter" data-open={open} inert={!open ? true : undefined}>
        <div className="phone-sheet__head">
          <span className="phone-sheet__title">Filter</span>
          <button ref={close} type="button" className="phone-sheet__close" aria-label="Close" onClick={() => setOpen(false)}>
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden="true">
              <path d="M5 5l14 14M19 5L5 19" />
            </svg>
          </button>
        </div>
        <div className="ui-filter-body">{children}</div>
      </div>
    </>
  );
}
