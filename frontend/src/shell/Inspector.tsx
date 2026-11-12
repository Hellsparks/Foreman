import type { ReactNode } from "react";
import EdgePanel from "../ui/EdgePanel";

interface InspectorProps {
  open: boolean;
  /** If omitted, no shell-level header is rendered (page provides its own via portal). */
  title?: string;
  typeLabel?: string;
  onClose: () => void;
  children?: ReactNode;
}

/**
 * Non-modal right-side Inspector panel.
 *
 * Per doc 02/03: selected record opens here; main view stays usable.
 * Below 760 px it is the right edge panel (doc 06 §2): details for the
 * selection, with a grab tab while something is selected.
 *
 * When `title` is omitted the shell header is suppressed and the page is
 * expected to render its own header inside the body (via React portal).
 */
export default function Inspector({
  open,
  title,
  typeLabel,
  onClose,
  children,
}: InspectorProps) {
  return (
    <EdgePanel side="right" label={title ?? "Details"} open={open} onOpenChange={(o) => !o && onClose()} available={open}>
    <aside
      className="inspector"
      data-open={open}
      aria-label={title ? `${title} inspector` : "Inspector"}
      aria-hidden={!open}
      inert={!open ? true : undefined}
    >
      {title !== undefined && (
        <div className="inspector__header">
          <div>
            {typeLabel && <p className="inspector__type">{typeLabel}</p>}
            <h2 className="inspector__title">{title}</h2>
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
      )}
      <div className="inspector__body" style={title === undefined ? { padding: 0 } : undefined}>
        {children}
      </div>
    </aside>
    </EdgePanel>
  );
}
