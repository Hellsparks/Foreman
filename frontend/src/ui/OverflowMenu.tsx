import { useEffect, useId, useRef, useState } from "react";

export interface OverflowItem {
  label: string;
  onSelect: () => void;
  disabled?: boolean;
  danger?: boolean;
}

interface OverflowMenuProps {
  items: OverflowItem[];
  /** Accessible name of the "⋯" trigger and the sheet. */
  label?: string;
}

/**
 * "⋯" button that opens a bottom sheet (the same sheet as the phone "More"
 * tab). Phone only: on desktop the callers render their inline buttons.
 * The trigger is a 44 px tap target; the sheet rows are 48 px.
 */
export default function OverflowMenu({ items, label = "More actions" }: OverflowMenuProps) {
  const [open, setOpen] = useState(false);
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const close = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    close.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
        trigger.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  return (
    <>
      <button
        ref={trigger}
        type="button"
        className="ui-overflow-btn"
        aria-label={label}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen((o) => !o)}
      >
        <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
          <circle cx="5" cy="12" r="2" />
          <circle cx="12" cy="12" r="2" />
          <circle cx="19" cy="12" r="2" />
        </svg>
      </button>
      {open && <div className="phone-sheet__scrim" onClick={() => setOpen(false)} />}
      <div id={id} className="phone-sheet" role="dialog" aria-label={label} data-open={open} inert={!open ? true : undefined}>
        <div className="phone-sheet__head">
          <span className="phone-sheet__title">{label}</span>
          <button ref={close} type="button" className="phone-sheet__close" aria-label="Close" onClick={() => setOpen(false)}>
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden="true">
              <path d="M5 5l14 14M19 5L5 19" />
            </svg>
          </button>
        </div>
        <ul className="phone-sheet__list" role="list">
          {items.map((it) => (
            <li key={it.label}>
              <button
                type="button"
                className={`ui-overflow-item${it.danger ? " ui-overflow-item--danger" : ""}`}
                disabled={it.disabled}
                onClick={() => {
                  setOpen(false);
                  it.onSelect();
                }}
              >
                {it.label}
              </button>
            </li>
          ))}
        </ul>
      </div>
    </>
  );
}
