import { useState, useCallback } from "react";
import { Outlet } from "react-router-dom";
import Rail from "./Rail";
import TopBar from "./TopBar";
import Inspector from "./Inspector";
import PhoneNav from "./PhoneNav";
import { useEventStream } from "../hooks/useEventStream";
import type { PrinterStatus } from "../api/types/PrinterStatus";
import "./Shell.css";

/** Live printer statuses keyed by printer_id. */
export type StatusMap = Record<number, PrinterStatus>;
/** Connection state per printer_id. */
export type ConnectionMap = Record<number, string>;

export interface ShellContext {
  statuses: StatusMap;
  connections: ConnectionMap;
  /** Open the inspector panel (shows whatever the page rendered into it). */
  openInspector: () => void;
  /** Close the inspector panel. */
  closeInspector: () => void;
  /** Whether the inspector is currently open. */
  inspectorOpen: boolean;
  /** ID of the DOM element that pages can portal their inspector content into. */
  inspectorBodyId: string;
}

const INSPECTOR_BODY_ID = "shell-inspector-body";

export default function Shell() {
  const [railCollapsed, setRailCollapsed] = useState(false);
  const [inspectorOpen, setInspectorOpen] = useState(false);

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

  const openInspector = useCallback(() => setInspectorOpen(true), []);
  const closeInspector = useCallback(() => setInspectorOpen(false), []);

  const context: ShellContext = {
    statuses,
    connections,
    openInspector,
    closeInspector,
    inspectorOpen,
    inspectorBodyId: INSPECTOR_BODY_ID,
  };

  return (
    <div className="shell">
      <Rail
        collapsed={railCollapsed}
        onToggle={() => setRailCollapsed((c) => !c)}
      />

      <div className="shell__body">
        <TopBar connectionState={connectionState} />

        <div className="shell__content">
          <main className="shell__work" id="main-content">
            <Outlet context={context} />
          </main>

          <Inspector
            open={inspectorOpen}
            onClose={closeInspector}
          >
            {/* Pages inject content here via React portal using INSPECTOR_BODY_ID */}
            <div id={INSPECTOR_BODY_ID} style={{ height: "100%" }} />
          </Inspector>
        </div>

        <PhoneNav />
      </div>
    </div>
  );
}
