/**
 * Phone: content in a full-height bottom sheet behind one button (doc 06 §1:
 * "a screen that needs a toolbar needs a sheet or a panel instead").
 * Desktop: the children render where they are, unchanged.
 */
import { useEffect, useRef, type ReactNode } from "react";
import { usePhone } from "../hooks/useMediaQuery";

interface Props {
  open: boolean;
  onClose: () => void;
  label: string;
  children: ReactNode;
}

export default function PhoneSheet({ open, onClose, label, children }: Props) {
  const phone = usePhone();
  const close = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!phone || !open) return;
    close.current?.focus();
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [phone, open, onClose]);
  if (!phone) return <>{children}</>;
  return (
    <>
      {open && <div className="phone-sheet__scrim" onClick={onClose} />}
      <div className="phone-sheet phone-sheet--tall" role="dialog" aria-label={label} data-open={open} inert={!open ? true : undefined}>
        <div className="phone-sheet__head">
          <span className="phone-sheet__title">{label}</span>
          <button ref={close} type="button" className="phone-sheet__close" aria-label="Close" onClick={onClose}>
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden="true">
              <path d="M5 5l14 14M19 5L5 19" />
            </svg>
          </button>
        </div>
        <div className="phone-sheet__body">{children}</div>
      </div>
    </>
  );
}
