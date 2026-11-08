import type { ReactNode } from "react";

interface InspectorProps {
  open: boolean;
  title?: string;
  typeLabel?: string;
  onClose: () => void;
  children?: ReactNode;
}

/**
 * Non-modal right-side Inspector panel.
 *
 * Per doc 02/03: selected record opens here; main view stays usable.
 * On phone (≤430px) this expands to full page — handled via CSS.
 */
export default function Inspector({
  open,
  title,
  typeLabel,
  onClose,
  children,
}: InspectorProps) {
  return (
    <aside
      className="inspector"
      data-open={open}
      aria-label={title ? `${title} inspector` : "Inspector"}
      aria-hidden={!open}
      inert={!open ? true : undefined}
    >
      <div className="inspector__header">
        <div>
          {typeLabel && <p className="inspector__type">{typeLabel}</p>}
          <h2 className="inspector__title">{title ?? "Inspector"}</h2>
        </div>
        <button
          type="button"
          className="inspector__close"
          aria-label="Close inspector"
          onClick={onClose}
        >
          <svg
            width="16"
            height="16"
            viewBox="0 0 16 16"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
            aria-hidden="true"
          >
            <line x1="3" y1="3" x2="13" y2="13" />
            <line x1="13" y1="3" x2="3" y2="13" />
          </svg>
        </button>
      </div>
      <div className="inspector__body">{children}</div>
    </aside>
  );
}
