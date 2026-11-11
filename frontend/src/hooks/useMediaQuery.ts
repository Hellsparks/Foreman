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
