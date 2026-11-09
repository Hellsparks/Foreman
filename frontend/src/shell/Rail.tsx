import { NavLink } from "react-router-dom";

// SVG path data for each nav item, from the UI reference (Main.dc.html).
// All use viewBox="0 0 24 24", stroke-width 1.8, round caps/joins.
const NAV_PATHS: Record<string, string> = {
  Fleet:    "M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z",
  Jobs:     "M8 6h12M8 12h12M8 18h12M4 6h.01M4 12h.01M4 18h.01",
  Library:  "M3 6h6l2 2h10v11H3z",
  Projects: "M12 3l9 5-9 5-9-5 9-5zM3 13l9 5 9-5",
  Filament: "M12 3a9 9 0 100 18 9 9 0 000-18zM12 9a3 3 0 100 6 3 3 0 000-6z",
  Service:  "M14 7a4 4 0 00-5 5l-6 6 3 3 6-6a4 4 0 005-5l-3 3-3-3z",
  Tools:    "M4 20l6-6M14 4l6 6-8 8-6-6zM9 9l6 6",
  Settings: "M12 9a3 3 0 100 6 3 3 0 000-6zM4 12h2M18 12h2M12 4v2M12 18v2M6.3 6.3l1.4 1.4M16.3 16.3l1.4 1.4M6.3 17.7l1.4-1.4M16.3 7.7l1.4-1.4",
};

function NavIcon({ name }: { name: string }) {
  return (
    <svg
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={NAV_PATHS[name]} />
    </svg>
  );
}

interface NavItem {
  to: string;
  label: string;
}

const NAV_ITEMS: NavItem[] = [
  { to: "/fleet",    label: "Fleet" },
  { to: "/jobs",     label: "Jobs" },
  { to: "/library",  label: "Library" },
  { to: "/projects", label: "Projects" },
  { to: "/filament", label: "Filament" },
  { to: "/service",  label: "Service" },
  { to: "/tools",    label: "Tools" },
  { to: "/settings", label: "Settings" },
];

interface RailProps {
  collapsed: boolean;
  onToggle: () => void;
}

export default function Rail({ collapsed, onToggle }: RailProps) {
  const navId = "rail-nav";

  return (
    <nav
      className="rail"
      data-collapsed={collapsed}
      aria-label="Main navigation"
    >
      {/* Brand block */}
      <div className="rail__header">
        <svg
          width="24"
          height="24"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          aria-hidden="true"
          className="rail__logo-icon"
        >
          <rect x="3" y="3" width="18" height="18" />
          <path d="M3 9h18M9 9v12" />
        </svg>
        <span className="rail__app-name">FOREMAN</span>
      </div>

      {/* Nav items */}
      <ul className="rail__nav" id={navId} role="list">
        {NAV_ITEMS.map(({ to, label }) => (
          <li key={to} className="rail__item">
            <NavLink
              to={to}
              className="rail__link"
              aria-label={collapsed ? label : undefined}
            >
              <span className="rail__icon">
                <NavIcon name={label} />
              </span>
              <span className="rail__label">{label}</span>
            </NavLink>
          </li>
        ))}
      </ul>

      {/* Collapse toggle */}
      <div className="rail__footer">
        <button
          type="button"
          className="rail__toggle"
          aria-expanded={!collapsed}
          aria-controls={navId}
          aria-label={collapsed ? "Expand navigation" : "Collapse navigation"}
          onClick={onToggle}
        >
          {/* chevron-left when expanded, chevron-right when collapsed */}
          <svg
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.8"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
            className="rail__toggle-icon"
          >
            <path d={collapsed ? "M9 6l6 6-6 6" : "M15 6l-6 6 6 6"} />
          </svg>
          <span className="rail__label">Collapse</span>
        </button>
      </div>
    </nav>
  );
}
