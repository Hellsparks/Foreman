/**
 * Phone header pieces (doc 06 §1, §4). On desktop each renders nothing (or its
 * children unchanged), so desktop layout is untouched.
 *  - PhoneTitle: the header title (e.g. "Jobs · History") instead of the section name.
 *  - HeaderActions: page buttons in the header; they replace the global search.
 *  - HeaderSearch: a search icon that expands to a field over the whole header.
 */
import { useEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { usePhone } from "../hooks/useMediaQuery";

const PAGE_SLOT = "topbar-page-slot";
const ACTIONS_SLOT = "topbar-actions-slot";

function useSlot(id: string): Element | null {
  const [el, setEl] = useState<Element | null>(null);
  useEffect(() => setEl(document.getElementById(id)), [id]);
  return el;
}

export function PhoneTitle({ children }: { children: ReactNode }) {
  const phone = usePhone();
  const slot = useSlot(PAGE_SLOT);
  if (!phone || !slot) return null;
  return createPortal(<p className="topbar__ptitle">{children}</p>, slot);
}

export function HeaderActions({ children }: { children: ReactNode }) {
  const phone = usePhone();
  const slot = useSlot(ACTIONS_SLOT);
  if (!phone || !slot) return null;
  return createPortal(children, slot);
}

interface SearchProps {
  value: string;
  onChange: (v: string) => void;
  /** Accessible name and placeholder ("Search this folder…"). */
  label: string;
}

/** Search icon → field over the whole header (Escape or ✕ closes; the query stays). */
export function HeaderSearch({ value, onChange, label }: SearchProps) {
  const [open, setOpen] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (open) input.current?.focus();
  }, [open]);
  if (!open) {
    return (
      <button type="button" className="topbar__search-toggle topbar__psearch-btn" aria-label={label} onClick={() => setOpen(true)} data-active={value ? "true" : undefined}>
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden="true">
          <circle cx="11" cy="11" r="7" />
          <path d="M20 20l-4-4" />
        </svg>
      </button>
    );
  }
  return (
    <div className="topbar__psearch">
      <input
        ref={input}
        type="search"
        className="topbar__psearch-input"
        aria-label={label}
        placeholder={label}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => e.key === "Escape" && setOpen(false)}
      />
      <button type="button" className="topbar__psearch-close" aria-label="Close search" onClick={() => setOpen(false)}>
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden="true">
          <path d="M5 5l14 14M19 5L5 19" />
        </svg>
      </button>
    </div>
  );
}
