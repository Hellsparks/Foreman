/**
 * Drag-out edge panel (doc 06 §2).
 *
 * Desktop: renders its children exactly as before (the existing sidebar or
 * inspector, no wrapper), so desktop layout does not change.
 * Phone (below 760 px): the children move into an overlay panel on the left or
 * right edge with a visible grab tab, vertically centred, with a chevron.
 *  - Left = navigation within the page; right = details for the selection.
 *  - Tap or drag the tab to open; drag it back, tap the backdrop or press
 *    Escape to close. The panel overlays content (never pushes it), max 85% of
 *    the viewport width.
 *  - Uncontrolled panels remember open/closed per page (`storageKey`).
 *  - The grip is 20 × 64 px; the button around it is 44 × 64 px (tap target).
 */
import { useCallback, useEffect, useId, useRef, useState, type ReactNode } from "react";
import { usePhone } from "../hooks/useMediaQuery";

interface Props {
  side: "left" | "right";
  /** Accessible name of the panel and its tab ("Locations", "Details"). */
  label: string;
  /** Remember open/closed under this key (uncontrolled panels). */
  storageKey?: string;
  /** Controlled open state (right panels follow the selection). */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  /** Show the tab at all (e.g. only while something is selected). Default true. */
  available?: boolean;
  /** Render the children on desktop too (default). False = phone-only panel. */
  desktop?: boolean;
  /** Close when a section, link or `[data-panel-close]` inside is chosen.
   *  Default: left panels (navigation) yes, right panels (details) no. */
  closeOnSelect?: boolean;
  children: ReactNode;
}

const KEY = (k: string) => `fw-panel:${k}`;

function readStored(key: string | undefined): boolean {
  if (!key) return false;
  try {
    return localStorage.getItem(KEY(key)) === "open";
  } catch {
    return false;
  }
}

/** Pixels a drag must travel to count as a drag (else it is a tap). */
const DRAG = 24;

export default function EdgePanel({
  side,
  label,
  storageKey,
  open: openProp,
  onOpenChange,
  available = true,
  desktop = true,
  closeOnSelect,
  children,
}: Props) {
  const closes = closeOnSelect ?? side === "left";
  const phone = usePhone();
  const controlled = openProp !== undefined;
  const [openState, setOpenState] = useState(() => readStored(storageKey));
  const open = controlled ? !!openProp : openState;
  const id = useId();
  const tab = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const drag = useRef<{ x: number; y: number } | null>(null);

  const setOpen = useCallback(
    (o: boolean) => {
      if (!controlled) {
        setOpenState(o);
        if (storageKey) {
          try {
            localStorage.setItem(KEY(storageKey), o ? "open" : "closed");
          } catch {
            /* private mode: not remembered */
          }
        }
      }
      onOpenChange?.(o);
    },
    [controlled, storageKey, onOpenChange],
  );

  useEffect(() => {
    if (!phone || !open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
        tab.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [phone, open, setOpen]);

  if (!phone) return desktop ? <>{children}</> : null;

  // Opening direction: towards the middle of the screen.
  const inward = side === "left" ? 1 : -1;
  const onDown = (e: React.PointerEvent) => {
    drag.current = { x: e.clientX, y: e.clientY };
  };
  const onUp = (e: React.PointerEvent) => {
    const d = drag.current;
    drag.current = null;
    if (!d) return;
    const dx = (e.clientX - d.x) * inward;
    if (Math.abs(e.clientY - d.y) > Math.abs(dx)) return;
    if (dx > DRAG) setOpen(true);
    else if (dx < -DRAG) setOpen(false);
  };

  return (
    <>
      {open && <div className="edge-scrim" onClick={() => setOpen(false)} />}
      <div
        ref={panel}
        id={id}
        className={`edge-panel edge-panel--${side}`}
        role="dialog"
        aria-label={label}
        data-open={open}
        inert={!open ? true : undefined}
        onPointerDown={onDown}
        onPointerUp={onUp}
        onClick={
          closes
            ? (e) => {
                if ((e.target as Element).closest('[role="tab"], a[href], [data-panel-close]')) setOpen(false);
              }
            : undefined
        }
      >
        {children}
      </div>
      {available && (
        <button
          ref={tab}
          type="button"
          className={`edge-tab edge-tab--${side}`}
          data-open={open}
          aria-label={open ? `Close ${label}` : `Open ${label}`}
          aria-expanded={open}
          aria-controls={id}
          onPointerDown={onDown}
          onPointerUp={(e) => {
            const d = drag.current;
            onUp(e);
            // A tap (no drag) toggles.
            if (d && Math.abs(e.clientX - d.x) <= DRAG && Math.abs(e.clientY - d.y) <= DRAG) setOpen(!open);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              setOpen(!open);
            }
          }}
        >
          <span className="edge-tab__grip" aria-hidden="true">
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4">
              <path d={(side === "left") !== open ? "M9 5l7 7-7 7" : "M15 5l-7 7 7 7"} />
            </svg>
          </span>
        </button>
      )}
    </>
  );
}
