/**
 * Finds text that does not fit its box: the "longest labels" check.
 *
 * - clipped: content is larger than a box that hides or spills it. A fail.
 * - truncated: single-line text cut with an ellipsis on purpose. Shown, not a fail.
 *
 * Scroll containers (overflow auto/scroll) are skipped: overflowing them is
 * their job.
 */
export interface ClipHit {
  el: HTMLElement;
  kind: "clipped" | "truncated";
  axis: "x" | "y";
  /** How many px do not fit. */
  by: number;
  what: string;
}

function describe(el: HTMLElement): string {
  const cls = typeof el.className === "string" && el.className.trim()
    ? "." + el.className.trim().split(/\s+/).join(".")
    : "";
  const text = (el.textContent ?? "").replace(/\s+/g, " ").trim().slice(0, 48);
  return `${el.tagName.toLowerCase()}${cls}${text ? ` “${text}”` : ""}`;
}

const SCROLLS = new Set(["auto", "scroll"]);

/** Inside a visually-hidden (≤ 1 px) box, e.g. a table header kept for screen readers. */
function srOnly(el: HTMLElement, root: HTMLElement, cache: Map<Element, boolean>): boolean {
  const p = el.parentElement;
  if (!p || p === root || !root.contains(p)) return false;
  const known = cache.get(p);
  if (known !== undefined) return known;
  const r = p.getBoundingClientRect();
  const hidden = (r.width <= 1 && r.height <= 1) || srOnly(p, root, cache);
  cache.set(p, hidden);
  return hidden;
}

export function scanClipping(root: HTMLElement): ClipHit[] {
  const hits: ClipHit[] = [];
  const all = [root, ...root.querySelectorAll<HTMLElement>("*")];
  const sr = new Map<Element, boolean>();
  for (const el of all) {
    // xterm draws its own text; data-clip-ignore opts a subtree out.
    if (el.closest("[data-clip-ignore], .xterm")) continue;
    // Only labels matter: boxes with no text of their own (icons, frames) are skipped.
    if (!(el.textContent ?? "").trim()) continue;
    if (el instanceof SVGElement || el.tagName === "svg") continue;
    const cs = getComputedStyle(el);
    if (cs.display === "none" || cs.visibility === "hidden") continue;
    // Visually-hidden screen-reader text is 1 px on purpose (and so is what is inside it).
    if (el.clientWidth <= 1 || el.clientHeight <= 1) continue;
    if (srOnly(el, root, sr)) continue;
    // Inputs/selects scroll their own text; images have no text.
    if (/^(INPUT|SELECT|TEXTAREA|IMG|CANVAS|VIDEO|TABLE|TBODY|THEAD|TR)$/.test(el.tagName)) continue;

    const overX = el.scrollWidth - el.clientWidth;
    const overY = el.scrollHeight - el.clientHeight;
    const ellipsis = cs.textOverflow === "ellipsis";

    if (overX > 1 && !SCROLLS.has(cs.overflowX)) {
      // An ellipsis is fine while a useful part of the text still shows;
      // cut down to a few characters it is as bad as clipping.
      const useful = el.clientWidth >= Math.min(64, el.scrollWidth * 0.5);
      hits.push({
        el,
        kind: ellipsis && cs.whiteSpace === "nowrap" && useful ? "truncated" : "clipped",
        axis: "x",
        by: overX,
        what: describe(el),
      });
    }
    // Glyphs may overhang a tight line-height by a pixel or two without
    // touching anything; only count that when the box hides its overflow.
    const tolerance = cs.overflowY === "visible" ? 3 : 1;
    if (overY > tolerance && !SCROLLS.has(cs.overflowY)) {
      // Hidden (cut off) or visible (spills over the next thing): both are wrong.
      hits.push({ el, kind: "clipped", axis: "y", by: overY, what: describe(el) });
    }
  }
  // Report the innermost box only when a parent and child flag the same overflow.
  return hits.filter(
    (h) => !hits.some((o) => o !== h && o.axis === h.axis && o.kind === h.kind && h.el.contains(o.el) && o.el !== h.el && Math.abs(o.by - h.by) < 2),
  );
}

const MARK = "data-clip-hit";

export function markHits(root: HTMLElement, hits: ClipHit[]): void {
  root.querySelectorAll(`[${MARK}]`).forEach((e) => e.removeAttribute(MARK));
  for (const h of hits) h.el.setAttribute(MARK, h.kind);
}
