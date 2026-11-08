import { NavLink } from "react-router-dom";

// 20×20 SVG icons, 1.5px stroke, square aesthetic per DESIGN.md.
// All use currentColor so they inherit the link's color state.
const Icons = {
  Fleet: () => (
    <svg
      width="20"
      height="20"
      viewBox="0 0 20 20"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden="true"
    >
      <rect x="2" y="2" width="7" height="7" rx="1" />
      <rect x="11" y="2" width="7" height="7" rx="1" />
      <rect x="2" y="11" width="7" height="7" rx="1" />
      <rect x="11" y="11" width="7" height="7" rx="1" />
    </svg>
  ),
  Jobs: () => (
    <svg
      width="20"
      height="20"
      viewBox="0 0 20 20"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden="true"
    >
      <line x1="3" y1="6" x2="17" y2="6" />
      <line x1="3" y1="10" x2="17" y2="10" />
      <line x1="3" y1="14" x2="11" y2="14" />
    </svg>
  ),
  Library: () => (
    <svg
      width="20"
      height="20"
      viewBox="0 0 20 20"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden="true"
    >
      <path d="M2 7h16v9a1 1 0 01-1 1H3a1 1 0 01-1-1V7z" />
      <path d="M2 7V5a1 1 0 011-1h4l2 2h8" />
    </svg>
  ),
  Projects: () => (
    <svg
      width="20"
      height="20"
      viewBox="0 0 20 20"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden="true"
    >
      <polygon points="10,2 18,6 10,10 2,6" />
      <polyline points="2,10 10,14 18,10" />
    </svg>
  ),
  Filament: () => (
    <svg
      width="20"
      height="20"
      viewBox="0 0 20 20"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden="true"
    >
      <circle cx="10" cy="10" r="7" />
      <circle cx="10" cy="10" r="3" />
      <line x1="10" y1="3" x2="10" y2="7" />
    </svg>
  ),
  Service: () => (
    <svg
      width="20"
      height="20"
      viewBox="0 0 20 20"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden="true"
    >
      <path d="M15 2a3 3 0 00-2.9 3.7L5.4 12.4a2 2 0 102.1 2.1l6.8-6.6A3 3 0 1015 2z" />
    </svg>
  ),
  Tools: () => (
    <svg
      width="20"
      height="20"
      viewBox="0 0 20 20"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden="true"
    >
      <line x1="3" y1="5" x2="17" y2="5" />
      <circle cx="8" cy="5" r="2" />
      <line x1="3" y1="11" x2="17" y2="11" />
      <circle cx="13" cy="11" r="2" />
      <line x1="3" y1="17" x2="17" y2="17" />
      <circle cx="7" cy="17" r="2" />
    </svg>
  ),
  Settings: () => (
    <svg
      width="20"
      height="20"
      viewBox="0 0 20 20"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden="true"
    >
      <circle cx="10" cy="10" r="3" />
      <path d="M10 2v2M10 16v2M2 10h2M16 10h2M4.2 4.2l1.4 1.4M14.4 14.4l1.4 1.4M4.2 15.8l1.4-1.4M14.4 5.6l1.4-1.4" />
    </svg>
  ),
  ChevronLeft: () => (
    <svg
      width="16"
      height="16"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden="true"
    >
      <polyline points="10,3 5,8 10,13" />
    </svg>
  ),
};

interface NavItem {
  to: string;
  label: string;
  Icon: () => React.JSX.Element;
}

const NAV_ITEMS: NavItem[] = [
  { to: "/fleet", label: "Fleet", Icon: Icons.Fleet },
  { to: "/jobs", label: "Jobs", Icon: Icons.Jobs },
  { to: "/library", label: "Library", Icon: Icons.Library },
  { to: "/projects", label: "Projects", Icon: Icons.Projects },
  { to: "/filament", label: "Filament", Icon: Icons.Filament },
  { to: "/service", label: "Service", Icon: Icons.Service },
  { to: "/tools", label: "Tools", Icon: Icons.Tools },
  { to: "/settings", label: "Settings", Icon: Icons.Settings },
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
      {/* App identity */}
      <div className="rail__header" aria-hidden="true">
        <div className="rail__logo">FM</div>
        <span className="rail__app-name">Foreman</span>
      </div>

      {/* Nav items */}
      <ul className="rail__nav" id={navId} role="list">
        {NAV_ITEMS.map(({ to, label, Icon }) => (
          <li key={to} className="rail__item">
            <NavLink
              to={to}
              className="rail__link"
              aria-label={collapsed ? label : undefined}
              aria-current={undefined} /* react-router sets this */
            >
              {({ isActive }) => (
                <>
                  <span
                    className="rail__icon"
                    style={{ color: isActive ? "var(--fw-accent-hover)" : undefined }}
                  >
                    <Icon />
                  </span>
                  <span className="rail__label">{label}</span>
                </>
              )}
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
          <span className="rail__toggle-icon">
            <Icons.ChevronLeft />
          </span>
        </button>
      </div>
    </nav>
  );
}
