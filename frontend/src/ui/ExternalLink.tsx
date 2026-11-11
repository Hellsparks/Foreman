/**
 * Link to an outside page: opens in a new tab, arrow-out icon (docs/ui-ref
 * README rule 5). Only http(s) URLs become links; anything else is shown as text.
 */
import type { MouseEvent, ReactNode } from "react";

export function safeHref(url: string | null | undefined): string | null {
  const u = (url ?? "").trim();
  if (!u) return null;
  const withScheme = /^[a-z][a-z0-9+.-]*:/i.test(u) ? u : `https://${u}`;
  try {
    const p = new URL(withScheme);
    return p.protocol === "http:" || p.protocol === "https:" ? p.href : null;
  } catch {
    return null;
  }
}

/** "www.sunlu.com" from "https://www.sunlu.com/products/x". */
export function hostLabel(url: string): string {
  try {
    return new URL(url).host.replace(/^www\./, "") || url;
  } catch {
    return url;
  }
}

export default function ExternalLink({
  href,
  children,
  label,
  className,
}: {
  href: string | null | undefined;
  /** Link text; defaults to the host name. */
  children?: ReactNode;
  /** Accessible name when the text alone is unclear ("Open product page"). */
  label?: string;
  /** Extra classes (e.g. to look like a button). */
  className?: string;
}) {
  const safe = safeHref(href);
  if (!safe) return href ? <span>{href}</span> : null;
  return (
    <a
      className={className ? `ext-link ${className}` : "ext-link"}
      href={safe}
      target="_blank"
      rel="noopener noreferrer"
      aria-label={label ? `${label} (opens in a new tab)` : undefined}
      title={safe}
      // Rows open the inspector on click; following the link should not.
      onClick={(e: MouseEvent) => e.stopPropagation()}
    >
      <span className="ext-link__text">{children ?? hostLabel(safe)}</span>
      <svg aria-hidden="true" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="square">
        <path d="M14 4h6v6M20 4l-9 9M18 14v6H4V6h6" />
      </svg>
    </a>
  );
}
