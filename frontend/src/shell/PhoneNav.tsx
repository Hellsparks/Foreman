import { useEffect, useId, useRef, useState } from "react";
import { NavLink, useLocation } from "react-router-dom";
import { OverlayPortal } from "../ui/Popover";

// Icons from PhoneFleet.dc.html (viewBox 24, stroke 1.8).
const ICONS: Record<string, string> = {
  Fleet: "M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z",
  Jobs: "M8 6h12M8 12h12M8 18h12M4 6h.01M4 12h.01M4 18h.01",
  Start: "M7 4l13 8-13 8z",
  Filament: "M12 3a9 9 0 100 18 9 9 0 000-18zM12 9a3 3 0 100 6 3 3 0 000-6z",
  Library: "M3 6h6l2 2h10v11H3z",
  Projects: "M12 3l9 5-9 5-9-5 9-5zM3 13l9 5 9-5",
  Service: "M14 7a4 4 0 00-5 5l-6 6 3 3 6-6a4 4 0 005-5l-3 3-3-3z",
  Tools: "M4 20l6-6M14 4l6 6-8 8-6-6zM9 9l6 6",
  Settings: "M12 9a3 3 0 100 6 3 3 0 000-6zM4 12h2M18 12h2M12 4v2M12 18v2M6.3 6.3l1.4 1.4M16.3 16.3l1.4 1.4M6.3 17.7l1.4-1.4M16.3 7.7l1.4-1.4",
};

function Icon({ name, size = 22, sw = 1.8 }: { name: string; size?: number; sw?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={sw} aria-hidden="true">
      <path d={ICONS[name]} />
    </svg>
  );
}

const MORE_ITEMS = [
  { to: "/library", label: "Library" },
  { to: "/projects", label: "Projects" },
  { to: "/service", label: "Service" },
  { to: "/tools", label: "Tools" },
  { to: "/settings", label: "Settings" },
];

/**
 * Bottom tab bar for phone (below 760 px), after PhoneFleet.dc.html and doc 06 §5:
 * Fleet · Jobs · Start print (accent, centre) · Filament · More.
 * Start print was Scan; Scan comes back under Filament when the devices exist.
 * "More" opens a sheet with the remaining sections. Hidden on desktop by CSS.
 */
export default function PhoneNav() {
  const [moreOpen, setMoreOpen] = useState(false);
  const { pathname } = useLocation();
  const sheetId = useId();
  const moreRef = useRef<HTMLButtonElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);

  // Any navigation closes the sheet.
  useEffect(() => setMoreOpen(false), [pathname]);

  useEffect(() => {
    if (!moreOpen) return;
    closeRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setMoreOpen(false);
        moreRef.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [moreOpen]);

  const inMore = MORE_ITEMS.some((i) => pathname === i.to || pathname.startsWith(i.to + "/"));

  return (
    <>
      <nav className="phone-nav" aria-label="Main">
        <NavLink to="/fleet" className="phone-nav__btn">
          <Icon name="Fleet" />Fleet
        </NavLink>
        <NavLink to="/jobs" className="phone-nav__btn">
          <Icon name="Jobs" />Jobs
        </NavLink>
        <NavLink to="/start" className="phone-nav__btn phone-nav__btn--start">
          <Icon name="Start" size={24} sw={2} />Start print
        </NavLink>
        <NavLink to="/filament" className="phone-nav__btn">
          <Icon name="Filament" />Filament
        </NavLink>
        <button
          ref={moreRef}
          type="button"
          className="phone-nav__btn"
          aria-haspopup="dialog"
          aria-expanded={moreOpen}
          aria-controls={sheetId}
          aria-current={inMore ? "page" : undefined}
          onClick={() => setMoreOpen((o) => !o)}
        >
          <svg width="22" height="22" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
            <circle cx="5" cy="12" r="2" /><circle cx="12" cy="12" r="2" /><circle cx="19" cy="12" r="2" />
          </svg>
          More
        </button>
      </nav>

      <OverlayPortal>
      {moreOpen && <div className="phone-sheet__scrim" onClick={() => setMoreOpen(false)} />}
      <div
        id={sheetId}
        className="phone-sheet"
        role="dialog"
        aria-label="More sections"
        data-open={moreOpen}
        inert={!moreOpen ? true : undefined}
      >
        <div className="phone-sheet__head">
          <span className="phone-sheet__title">More</span>
          <button ref={closeRef} type="button" className="phone-sheet__close" aria-label="Close" onClick={() => setMoreOpen(false)}>
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden="true">
              <path d="M5 5l14 14M19 5L5 19" />
            </svg>
          </button>
        </div>
        <ul className="phone-sheet__list" role="list">
          {MORE_ITEMS.map(({ to, label }) => (
            <li key={to}>
              <NavLink to={to} className="phone-sheet__link">
                <Icon name={label} />
                <span>{label}</span>
              </NavLink>
            </li>
          ))}
        </ul>
      </div>
      </OverlayPortal>
    </>
  );
}
