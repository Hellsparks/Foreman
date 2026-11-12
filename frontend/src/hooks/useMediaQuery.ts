import { useEffect, useState } from "react";

/** The phone layout applies below this width (px). Keep in sync with the
 *  `@media (max-width: 759px)` blocks in shell/Shell.css and the page CSS. */
export const PHONE_MAX = 759;
export const PHONE_QUERY = `(max-width: ${PHONE_MAX}px)`;

/** Live match state of a CSS media query. */
export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(() => window.matchMedia(query).matches);
  useEffect(() => {
    const mq = window.matchMedia(query);
    const on = () => setMatches(mq.matches);
    on();
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, [query]);
  return matches;
}

/** True below the 760 px breakpoint, where the shell changes shape. */
export function usePhone(): boolean {
  return useMediaQuery(PHONE_QUERY);
}

/** List + inspector side by side from this width; below it the inspector is an
 *  overlay panel (doc 06 §11). From content: rail 176 + gutters 64 + the
 *  narrowest useful list (~560: a 3-column table or two 280 px cards) +
 *  inspector 400 ≈ 1200. Keep in sync with the 1199 px queries in the CSS. */
export const SIDE_BY_SIDE_MIN = 1200;
export const OVERLAY_INSPECTOR_QUERY = `(max-width: ${SIDE_BY_SIDE_MIN - 1}px)`;
