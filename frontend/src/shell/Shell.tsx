import { useState, useCallback } from "react";
import { Outlet, NavLink } from "react-router-dom";
import Rail from "./Rail";
import TopBar from "./TopBar";
import Inspector from "./Inspector";
import { useAppearance } from "../hooks/useAppearance";
import { useEventStream } from "../hooks/useEventStream";
import type { PrinterStatus } from "../api/types/PrinterStatus";
import "./Shell.css";

/** Live printer statuses keyed by printer_id. */
export type StatusMap = Record<number, PrinterStatus>;
/** Connection state per printer_id. */
export type ConnectionMap = Record<number, string>;

export default function Shell() {
  const [railCollapsed, setRailCollapsed] = useState(false);
  const [inspectorOpen, setInspectorOpen] = useState(false);
  const { theme, mode, setTheme, setMode } = useAppearance();

  // Live printer data from the event bus.
  const [statuses, setStatuses] = useState<StatusMap>({});
  const [connections, setConnections] = useState<ConnectionMap>({});

  const handleEvent = useCallback(
    (event: { type: string; [key: string]: unknown }) => {
      if (event.type === "printer_status") {
        const pid = event.printer_id as number;
        const status = event.status as PrinterStatus;
        setStatuses((prev) => ({ ...prev, [pid]: status }));
      } else if (event.type === "printer_connection") {
        const pid = event.printer_id as number;
        const conn = event.event as { kind: string };
        setConnections((prev) => ({ ...prev, [pid]: conn.kind }));
      }
    },
    []
  );

  const { connectionState } = useEventStream(handleEvent);

  return (
    <div className="shell">
      <Rail
        collapsed={railCollapsed}
        onToggle={() => setRailCollapsed((c) => !c)}
      />

      <div className="shell__body">
        <TopBar
          connectionState={connectionState}
          theme={theme}
          mode={mode}
          onThemeChange={setTheme}
          onModeChange={setMode}
          attentionCount={0}
        />

        <div className="shell__content">
          <main className="shell__work" id="main-content">
            <Outlet
              context={{
                openInspector: () => setInspectorOpen(true),
                statuses,
                connections,
              }}
            />
          </main>

          <Inspector
            open={inspectorOpen}
            onClose={() => setInspectorOpen(false)}
          />
        </div>

        <PhoneNav />
      </div>
    </div>
  );
}

/** Bottom navigation for phone (≤430px). Visible only via CSS media query. */
function PhoneNav() {
  return (
    <nav className="phone-nav" aria-label="Phone navigation">
      <NavLink to="/fleet" className="phone-nav__btn" aria-label="Fleet">
        <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
          <rect x="2" y="2" width="7" height="7" rx="1" />
          <rect x="11" y="2" width="7" height="7" rx="1" />
          <rect x="2" y="11" width="7" height="7" rx="1" />
          <rect x="11" y="11" width="7" height="7" rx="1" />
        </svg>
        <span>Fleet</span>
      </NavLink>
      <NavLink to="/jobs" className="phone-nav__btn" aria-label="Jobs">
        <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
          <line x1="3" y1="6" x2="17" y2="6" />
          <line x1="3" y1="10" x2="17" y2="10" />
          <line x1="3" y1="14" x2="11" y2="14" />
        </svg>
        <span>Jobs</span>
      </NavLink>
      <NavLink to="/filament" className="phone-nav__btn" aria-label="Filament">
        <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
          <circle cx="10" cy="10" r="7" />
          <circle cx="10" cy="10" r="3" />
        </svg>
        <span>Filament</span>
      </NavLink>
      <NavLink to="/settings" className="phone-nav__btn" aria-label="More">
        <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
          <circle cx="5" cy="10" r="1.5" fill="currentColor" stroke="none" />
          <circle cx="10" cy="10" r="1.5" fill="currentColor" stroke="none" />
          <circle cx="15" cy="10" r="1.5" fill="currentColor" stroke="none" />
        </svg>
        <span>More</span>
      </NavLink>
    </nav>
  );
}
