/**
 * Popover + Menu (doc 06 §8 "Overlays portal to the body").
 *
 * Every floating thing (card "⋯" menus, dropdowns, combo-box lists, the bell
 * panel) renders into one overlay root on <body> through a portal, so no
 * parent's `overflow` or `transform` can clip it. It is `position: fixed`
 * against its trigger, flips to the other side and alignment when it would
 * leave the viewport, and repositions on scroll (any scroller) and resize.
 * Outside pointer-down or Escape closes it; Escape returns focus to the trigger.
 */
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
  type RefObject,
} from "react";
import { createPortal } from "react-dom";

const ROOT_ID = "fw-overlay-root";

/** The single overlay root (created on first use). */
export function overlayRoot(): HTMLElement {
  let el = document.getElementById(ROOT_ID);
  if (!el) {
    el = document.createElement("div");
    el.id = ROOT_ID;
    document.body.appendChild(el);
  }
  return el;
}

/** Portal children into the overlay root (sheets, scrims). */
export function OverlayPortal({ children }: { children: ReactNode }) {
  return createPortal(children, overlayRoot());
}

export type Side = "bottom" | "top";
export type Align = "start" | "end";

const MARGIN = 8;

export interface Placed {
  top: number;
  left: number;
  maxHeight: number;
  side: Side;
  align: Align;
}

/**
 * Where a box of `size` goes next to `anchor` inside a `vw` × `vh` viewport.
 * Prefers `side`/`align`; flips when the preferred one does not fit and the
 * other has more room; then clamps inside the viewport. Pure (unit-tested).
 */
export function place(
  anchor: { top: number; bottom: number; left: number; right: number },
  size: { width: number; height: number },
  vw: number,
  vh: number,
  side: Side = "bottom",
  align: Align = "end",
  gap = 4,
): Placed {
  const below = vh - anchor.bottom - gap - MARGIN;
  const above = anchor.top - gap - MARGIN;
  let s = side;
  if (s === "bottom" && size.height > below && above > below) s = "top";
  else if (s === "top" && size.height > above && below > above) s = "bottom";
  const room = s === "bottom" ? below : above;
  const h = Math.min(size.height, Math.max(room, 0));
  const top = s === "bottom" ? anchor.bottom + gap : anchor.top - gap - h;

  let a = align;
  const endLeft = anchor.right - size.width;
  const startLeft = anchor.left;
  if (a === "end" && endLeft < MARGIN && startLeft + size.width <= vw - MARGIN) a = "start";
  else if (a === "start" && startLeft + size.width > vw - MARGIN && endLeft >= MARGIN) a = "end";
  let left = a === "end" ? endLeft : startLeft;
  left = Math.max(MARGIN, Math.min(left, vw - MARGIN - size.width));
  return { top: Math.max(MARGIN, top), left, maxHeight: Math.max(room, 0), side: s, align: a };
}

interface PopoverProps {
  anchorRef: RefObject<HTMLElement | null>;
  open: boolean;
  onClose: () => void;
  side?: Side;
  align?: Align;
  /** Same width as the trigger (combo-box lists). */
  matchWidth?: boolean;
  /** Below 760 px: a full-height sheet instead of an anchored box (the bell). */
  sheetOnPhone?: boolean;
  role?: string;
  label?: string;
  id?: string;
  className?: string;
  /** Move focus into the popover when it opens (menus, dialogs). Combo boxes keep it in the input. */
  autoFocus?: boolean;
  children: ReactNode;
}

export default function Popover({
  anchorRef,
  open,
  onClose,
  side = "bottom",
  align = "end",
  matchWidth,
  sheetOnPhone,
  role = "dialog",
  label,
  id,
  className,
  autoFocus = true,
  children,
}: PopoverProps) {
  const box = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<Placed | null>(null);
  const [width, setWidth] = useState<number | undefined>(undefined);

  const update = useCallback(() => {
    const a = anchorRef.current;
    const b = box.current;
    if (!a || !b) return;
    const r = a.getBoundingClientRect();
    // Measure the natural height: drop the previous max-height first.
    b.style.maxHeight = "none";
    const w = matchWidth ? r.width : b.offsetWidth;
    const p = place(r, { width: w, height: b.scrollHeight }, window.innerWidth, window.innerHeight, side, align);
    setWidth(matchWidth ? r.width : undefined);
    setPos(p);
  }, [anchorRef, matchWidth, side, align]);

  useLayoutEffect(() => {
    if (!open) {
      setPos(null);
      return;
    }
    update();
  }, [open, update]);

  const updateRef = useRef(update);
  updateRef.current = update;
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  useEffect(() => {
    if (!open) return;
    const update = () => updateRef.current();
    const onClose = () => closeRef.current();
    const onScroll = () => update();
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onScroll);
    const ro = typeof ResizeObserver !== "undefined" ? new ResizeObserver(() => update()) : null;
    if (box.current) ro?.observe(box.current);
    const onDown = (e: PointerEvent) => {
      const t = e.target as Node;
      if (box.current?.contains(t) || anchorRef.current?.contains(t)) return;
      onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onClose();
        anchorRef.current?.focus();
      }
    };
    window.addEventListener("pointerdown", onDown, true);
    window.addEventListener("keydown", onKey);
    if (autoFocus) {
      const first = box.current?.querySelector<HTMLElement>('[role="menuitem"], button, a[href], input, [tabindex]');
      (first ?? box.current)?.focus({ preventScroll: true });
    }
    return () => {
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onScroll);
      window.removeEventListener("pointerdown", onDown, true);
      window.removeEventListener("keydown", onKey);
      ro?.disconnect();
    };
  }, [open, anchorRef, autoFocus]);

  if (!open) return null;
  const style: CSSProperties = pos
    ? ({ top: pos.top, left: pos.left, maxHeight: pos.maxHeight, width, "--pop-max": `${pos.maxHeight}px` } as CSSProperties)
    : { top: 0, left: 0, visibility: "hidden", width };
  return createPortal(
    <div
      ref={box}
      id={id}
      role={role}
      aria-label={label}
      tabIndex={-1}
      className={`ui-pop${sheetOnPhone ? " ui-pop--sheet" : ""}${className ? ` ${className}` : ""}`}
      data-side={pos?.side}
      data-align={pos?.align}
      style={style}
      // React events bubble through portals to the trigger's ancestors (a
      // clickable card, a draggable row): keep them inside the popover.
      onClick={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      onPointerDown={(e) => e.stopPropagation()}
      onDragStart={(e) => e.stopPropagation()}
    >
      {children}
    </div>,
    overlayRoot(),
  );
}

// ── Menu ───────────────────────────────────────────────────────────────────

export interface MenuItem {
  label: ReactNode;
  onSelect?: () => void;
  href?: string;
  disabled?: boolean;
  danger?: boolean;
  /** A non-interactive group label ("MOVE TO"). */
  heading?: boolean;
  /** Indented (items under a heading). */
  sub?: boolean;
  key?: string;
}

/** Up/down/Home/End move between menu items (WAI-ARIA menu pattern). */
function onMenuKeys(e: ReactKeyboardEvent<HTMLDivElement>) {
  const items = [...e.currentTarget.querySelectorAll<HTMLElement>('[role="menuitem"]:not([aria-disabled="true"])')];
  const i = items.indexOf(document.activeElement as HTMLElement);
  let n = -1;
  if (e.key === "ArrowDown") n = (i + 1) % items.length;
  else if (e.key === "ArrowUp") n = (i - 1 + items.length) % items.length;
  else if (e.key === "Home") n = 0;
  else if (e.key === "End") n = items.length - 1;
  if (n >= 0 && items[n]) {
    e.preventDefault();
    items[n].focus();
  }
}

interface MenuProps {
  /** Accessible name of the menu. */
  label: string;
  items: MenuItem[];
  /** Renders the trigger; spread the props onto the button. */
  trigger: (p: {
    ref: RefObject<HTMLButtonElement | null>;
    "aria-haspopup": "menu";
    "aria-expanded": boolean;
    onClick: (e: React.MouseEvent) => void;
  }) => ReactNode;
  side?: Side;
  align?: Align;
  /** Controlled open state (optional). */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}

export function Menu({ label, items, trigger, side, align, open: openProp, onOpenChange }: MenuProps) {
  const [openState, setOpenState] = useState(false);
  const open = openProp ?? openState;
  const setOpen = (o: boolean) => {
    if (openProp === undefined) setOpenState(o);
    onOpenChange?.(o);
  };
  const ref = useRef<HTMLButtonElement>(null);
  return (
    <>
      {trigger({
        ref,
        "aria-haspopup": "menu",
        "aria-expanded": open,
        onClick: (e) => {
          e.stopPropagation();
          setOpen(!open);
        },
      })}
      <Popover anchorRef={ref} open={open} onClose={() => setOpen(false)} role="menu" label={label} side={side} align={align} className="ui-menu">
        <div className="ui-menu__list" onKeyDown={onMenuKeys}>
          {items.map((it, i) =>
            it.heading ? (
              <div key={it.key ?? i} className="ui-menu__heading" role="presentation">
                {it.label}
              </div>
            ) : it.href ? (
              <a
                key={it.key ?? i}
                role="menuitem"
                className={`ui-menu__item${it.sub ? " ui-menu__item--sub" : ""}`}
                href={it.href}
                target="_blank"
                rel="noopener noreferrer"
                onClick={() => setOpen(false)}
              >
                {it.label}
              </a>
            ) : (
              <button
                key={it.key ?? i}
                type="button"
                role="menuitem"
                aria-disabled={it.disabled || undefined}
                disabled={it.disabled}
                className={`ui-menu__item${it.sub ? " ui-menu__item--sub" : ""}${it.danger ? " ui-menu__item--danger" : ""}`}
                onClick={() => {
                  setOpen(false);
                  it.onSelect?.();
                }}
              >
                {it.label}
              </button>
            ),
          )}
        </div>
      </Popover>
    </>
  );
}
