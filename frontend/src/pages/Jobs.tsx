/** Jobs page (docs/ui-ref/Jobs.dc.html). Queue board (task 3.5); History and Stats
 *  (task 3.6, JobsHistory.tsx). */
import { useMemo, useState } from "react";
import { Link, useOutletContext } from "react-router-dom";
import type { QueueLane } from "../api/queue";
import type { ShellContext } from "../shell/Shell";
import { PhoneTitle, EdgePanel, ToastStack, useToasts } from "../ui";
import { JobsHistory, JobsStats } from "./JobsHistory";
import { QueueBoard, useQueueBoard } from "./QueueBoard";
import "./Jobs.css";

type Tab = "queue" | "history" | "stats";

export default function Jobs() {
  const { statuses } = useOutletContext<ShellContext>();
  const [tab, setTab] = useState<Tab>("queue");
  const { board, loadErr, load } = useQueueBoard();
  const { toasts, addToast, removeToast } = useToasts();

  const lanes = board?.lanes ?? [];
  const allCards = lanes.flatMap((l) => l.items);
  const waitingCards = allCards.filter((c) => c.item.status === "waiting");

  const headline = useMemo(() => {
    const stOf = (l: QueueLane) => (l.printer_id != null ? (statuses[l.printer_id] ?? l.status) : null);
    const printing = lanes.filter((l) => stOf(l)?.state === "printing").length;
    const plate = lanes.filter(
      (l) => l.printer_id != null && !l.plate_clear && !["printing", "paused"].includes(stOf(l)?.state ?? ""),
    ).length;
    const grams = waitingCards.reduce((a, c) => a + (c.grams ?? 0), 0);
    return [
      `${waitingCards.length} queued`,
      `${printing} printing`,
      `${plate} waiting for plate clear`,
      `~${Math.round(grams)} g of filament queued`,
    ].join(" · ");
  }, [lanes, statuses, waitingCards]);

  const tabs: [Tab, string][] = [
    ["queue", "Queue"],
    ["history", "History"],
    ["stats", "Stats"],
  ];

  return (
    <div className="jobs-page">
      <header className="jobs-head">
        <div className="jobs-head__row">
          <div className="jobs-head__text">
            <h1 className="jobs-head__title phone-dup-title">Jobs</h1>
            <p className="jobs-head__sub">{headline}</p>
          </div>
          <Link to="/library" className="jobs-add">
            Add job
          </Link>
        </div>
        <PhoneTitle>
          Jobs <span>· {tabs.find(([id]) => id === tab)?.[1]}</span>
        </PhoneTitle>
        <EdgePanel side="left" label="Jobs sections" storageKey="jobs.sections">
        <div role="tablist" aria-label="Jobs sections" className="jobs-tabs">
          {tabs.map(([id, label]) => (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={tab === id}
              className={`jobs-tab${tab === id ? " jobs-tab--on" : ""}`}
              onClick={() => setTab(id)}
            >
              {label}
            </button>
          ))}
        </div>
        </EdgePanel>
      </header>

      {tab === "history" && <JobsHistory notify={addToast} />}
      {tab === "stats" && <JobsStats />}

      {tab === "queue" && (
        <QueueBoard board={board} loadErr={loadErr} load={load} addToast={addToast} />
      )}
      <ToastStack toasts={toasts} onDismiss={removeToast} />
    </div>
  );
}
