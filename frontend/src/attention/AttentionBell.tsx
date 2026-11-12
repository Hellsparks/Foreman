/**
 * Bell + count in the top bar (desktop) / header (phone) that opens the open
 * Attention items with their actions (§3, doc 06).
 * Desktop: anchored panel ≈420 px wide, max-height 70vh, scrolls.
 * Phone: full-height sheet. "View all" opens the Attention page.
 * Escape, the close button or a click outside closes it.
 */
import { useEffect, useId, useRef, useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { api } from "../api/client";
import AttentionList from "./AttentionList";
import { useAttention } from "./useAttention";

export default function AttentionBell() {
  const { items, remove } = useAttention();
  const [open, setOpen] = useState(false);
  const [names, setNames] = useState<Record<number, string>>({});
  const id = useId();
  const btn = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const n = items.length;

  // Any navigation closes the panel.
  useEffect(() => setOpen(false), [pathname]);

  useEffect(() => {
    if (!open) return;
    api
      .listPrinters()
      .then((ps) => setNames(Object.fromEntries(ps.map((p) => [p.id, p.name]))))
      .catch(() => {});
    panel.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
        btn.current?.focus();
      }
    };
    const onDown = (e: PointerEvent) => {
      const t = e.target as Node;
      if (!panel.current?.contains(t) && !btn.current?.contains(t)) setOpen(false);
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("pointerdown", onDown);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("pointerdown", onDown);
    };
  }, [open]);

  return (
    <div className="attn-bell">
      <button
        ref={btn}
        type="button"
        className="topbar__btn topbar__btn--attention"
        aria-label={`Attention — ${n} item${n === 1 ? "" : "s"} need action`}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen((o) => !o)}
      >
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden="true">
          <path d="M6 16V11a6 6 0 0112 0v5l2 2H4l2-2zM10 21h4" />
        </svg>
        <span className="topbar__attention-count">{n}</span>
      </button>
      {open && (
        <div ref={panel} id={id} className="attn-pop" role="dialog" aria-label="Attention" tabIndex={-1}>
          <div className="attn-pop__head">
            <span className="attn-pop__title">Attention · {n}</span>
            <button type="button" className="attn-pop__close" aria-label="Close" onClick={() => setOpen(false)}>
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden="true">
                <path d="M5 5l14 14M19 5L5 19" />
              </svg>
            </button>
          </div>
          <div className="attn-pop__body">
            <AttentionList items={items} printerNames={names} onResolved={remove} onNavigate={() => setOpen(false)} />
          </div>
          <button
            type="button"
            className="attn-pop__all"
            onClick={() => {
              setOpen(false);
              navigate("/attention");
            }}
          >
            View all
          </button>
        </div>
      )}
    </div>
  );
}
