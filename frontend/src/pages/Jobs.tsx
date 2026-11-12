/** Jobs page (docs/ui-ref/Jobs.dc.html). Queue board (task 3.5); History and Stats
 *  (task 3.6, JobsHistory.tsx). */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link, useOutletContext } from "react-router-dom";
import { queue, type QueueBoard, type QueueCard, type QueueLane } from "../api/queue";
import type { PrinterStatus } from "../api/types/PrinterStatus";
import { useEventStream } from "../hooks/useEventStream";
import { useSignedUrl } from "../hooks/useSignedUrl";
import type { ShellContext } from "../shell/Shell";
import { PhoneTitle, EdgePanel, StatusChip, ToastStack, useToasts, type Tone } from "../ui";
import { JobsHistory, JobsStats } from "./JobsHistory";
import "./Jobs.css";

type Tab = "queue" | "history" | "stats";
type LaneKey = number | null;

const TOPICS = ["queue", "printer.status", "printer.connection", "attention.added", "attention.cleared"];

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

function fmtTime(secs: number | null): string | null {
  if (secs == null) return null;
  const m = Math.round(secs / 60);
  if (m < 60) return `${m} m`;
  return `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, "0")} m`;
}

function laneKey(l: QueueLane): string {
  return l.printer_id == null ? "fleet" : String(l.printer_id);
}

/** Lane header chip + "now" line from the live printer status. */
function laneState(lane: QueueLane, st: PrinterStatus | undefined): { tone: Tone; word: string; now: string } {
  if (lane.printer_id == null) {
    return { tone: "info", word: "ANY COMPATIBLE", now: "Feeds idle printers as plates are cleared" };
  }
  const state = st?.state ?? "offline";
  if (state === "printing" || state === "paused") {
    const pct = st?.progress != null ? ` ${Math.round(st.progress * 100)}%` : "";
    const left = fmtTime(st?.remaining_time ?? null);
    const now = [st?.filename, left ? `${left} left` : null].filter(Boolean).join(" · ") || "Printing";
    return state === "paused"
      ? { tone: "warn", word: `‖ PAUSED${pct}`, now: st?.display_message || now }
      : { tone: "info", word: `▶ PRINTING${pct}`, now };
  }
  if (state === "error") return { tone: "bad", word: "✖ ERROR", now: st?.display_message || "Printer reports an error" };
  if (state === "offline") return { tone: "off", word: "○ OFFLINE", now: "Not connected" };
  if (!lane.plate_clear) return { tone: "warn", word: "◐ CLEAR PLATE", now: "Print done · waiting for you" };
  return {
    tone: "ok",
    word: "● IDLE",
    now: lane.auto_start ? "Will take the next job" : "Auto-start off · use Start next now",
  };
}

function Thumb({ path, colour }: { path: string | null; colour: string | null }) {
  const url = useSignedUrl(path);
  const [broken, setBroken] = useState(false);
  if (!url || broken) {
    return <span className="jobs-card__thumb jobs-card__thumb--blank" style={colour ? { background: colour } : undefined} />;
  }
  return <img className="jobs-card__thumb" src={url} alt="" width={48} height={36} onError={() => setBroken(true)} />;
}

export default function Jobs() {
  const { statuses } = useOutletContext<ShellContext>();
  const [tab, setTab] = useState<Tab>("queue");
  const [board, setBoard] = useState<QueueBoard | null>(null);
  const [loadErr, setLoadErr] = useState<string | null>(null);
  const [menu, setMenu] = useState<number | null>(null);
  const [drag, setDrag] = useState<number | null>(null);
  const [dropAt, setDropAt] = useState<{ lane: string; index: number } | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const { toasts, addToast, removeToast } = useToasts();

  const load = useCallback(async () => {
    try {
      setBoard(await queue.board());
      setLoadErr(null);
    } catch (e) {
      setLoadErr(errText(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // Live: refetch (debounced) on queue / attention events.
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const onEvent = useCallback(
    (ev: { type: string }) => {
      if (ev.type === "queue_changed" || ev.type.startsWith("attention_")) {
        if (timer.current) clearTimeout(timer.current);
        timer.current = setTimeout(() => void load(), 150);
      }
    },
    [load],
  );
  useEventStream(onEvent, TOPICS);
  useEffect(() => () => void (timer.current && clearTimeout(timer.current)), []);

  // Close the card menu on outside click / Esc.
  useEffect(() => {
    if (menu == null) return;
    const close = (e: MouseEvent | KeyboardEvent) => {
      if (e instanceof KeyboardEvent && e.key !== "Escape") return;
      if (e instanceof MouseEvent && (e.target as HTMLElement).closest(".jobs-menu, .jobs-card__more")) return;
      setMenu(null);
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", close);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", close);
    };
  }, [menu]);

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

  const laneName = (k: LaneKey) => lanes.find((l) => l.printer_id === k)?.name ?? "queue";
  const waitingIndex = (c: QueueCard) => {
    const lane = lanes.find((l) => l.printer_id === c.item.printer_id);
    return lane ? lane.items.filter((x) => x.item.status === "waiting").findIndex((x) => x.item.id === c.item.id) : 0;
  };

  const doMove = async (c: QueueCard, to: LaneKey, index: number, undoable = true) => {
    const from = c.item.printer_id;
    const fromIdx = waitingIndex(c);
    setMenu(null);
    try {
      await queue.move(c.item.id, to, index);
      await load();
      if (undoable) {
        const where = to === from ? `position ${index + 1}` : laneName(to);
        addToast(`${c.item.file_name} → ${where}`, () => void doMove({ ...c, item: { ...c.item, printer_id: to } }, from, fromIdx, false));
      }
    } catch (e) {
      addToast(`Couldn't move ${c.item.file_name}: ${errText(e)}`);
    }
  };

  const doRemove = async (c: QueueCard) => {
    setMenu(null);
    const lane = c.item.printer_id;
    const idx = waitingIndex(c);
    try {
      const r = await queue.remove(c.item.id);
      await load();
      if (r.removed === "cancelled") {
        addToast(`Cancelled ${c.item.file_name} — the upload stops before printing`);
      } else {
        addToast(`Removed ${c.item.file_name}`, () => {
          void (async () => {
            try {
              // Undo puts back what was already accepted (manual swaps included).
              const p = await queue.add(c.item.file_id, lane, true);
              if (c.item.status === "waiting") await queue.move(p.item_id, lane, Math.max(idx, 0));
              await load();
            } catch (e) {
              addToast(`Undo failed: ${errText(e)}`);
            }
          })();
        });
      }
    } catch (e) {
      addToast(`Couldn't remove ${c.item.file_name}: ${errText(e)}`);
    }
  };

  const startNext = async (lane: QueueLane) => {
    if (lane.printer_id == null) return;
    setBusy(`start-${lane.printer_id}`);
    try {
      const r = await queue.startNext(lane.printer_id, {});
      addToast(r.message);
      await load();
    } catch (e) {
      addToast(`${lane.name}: ${errText(e)}`);
    } finally {
      setBusy(null);
    }
  };

  const plateCleared = async (lane: QueueLane) => {
    if (lane.printer_id == null) return;
    setBusy(`plate-${lane.printer_id}`);
    try {
      await queue.plateCleared(lane.printer_id);
      addToast(`Plate cleared on ${lane.name}`);
      await load();
    } catch (e) {
      addToast(`${lane.name}: ${errText(e)}`);
    } finally {
      setBusy(null);
    }
  };

  // ── drag & drop ───────────────────────────────────────────────────────────
  const dragCard = drag != null ? allCards.find((c) => c.item.id === drag) : undefined;
  const onDrop = (lane: QueueLane) => {
    const target = dropAt && dropAt.lane === laneKey(lane) ? dropAt.index : lane.items.filter((x) => x.item.status === "waiting").length;
    const c = dragCard;
    setDrag(null);
    setDropAt(null);
    if (!c) return;
    let index = target;
    // Same lane, moving down: the card's own slot disappears first.
    if (c.item.printer_id === lane.printer_id) {
      const cur = waitingIndex(c);
      if (cur >= 0 && cur < index) index -= 1;
      if (cur === index && c.item.status === "waiting") return;
    }
    void doMove(c, lane.printer_id, index);
  };

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
        <main className="jobs-main">
          <p className="jobs-hint">
            Drag cards between lanes, or use ⋯ on a card. The fleet lane feeds any compatible printer, preferring one that
            already has the right filament loaded. A printer starts the next job only after its plate is cleared.
          </p>
          {loadErr && <p className="jobs-err">{loadErr}</p>}
          <div className="jobs-board">
            <div className="jobs-lanes">
              {lanes.map((lane) => {
                const key = laneKey(lane);
                const st = lane.printer_id != null ? (statuses[lane.printer_id] ?? lane.status ?? undefined) : undefined;
                const ls = laneState(lane, st);
                const waiting = lane.items.filter((x) => x.item.status === "waiting");
                const active = lane.items.some((x) => x.item.status === "sending" || x.item.status === "printing");
                const canStart =
                  lane.printer_id != null && !active && (waiting.length > 0 || (lanes[0]?.items.some((x) => x.item.status === "waiting") ?? false));
                const isDrop = drag != null && dropAt?.lane === key;
                return (
                  <section
                    key={key}
                    aria-label={`${lane.name} queue`}
                    className={`jobs-lane${lane.printer_id == null ? " jobs-lane--fleet" : ""}${isDrop ? " jobs-lane--drop" : ""}`}
                    onDragOver={(e) => {
                      if (drag == null) return;
                      e.preventDefault();
                      if (dropAt?.lane !== key) setDropAt({ lane: key, index: waiting.length });
                    }}
                    onDrop={(e) => {
                      e.preventDefault();
                      onDrop(lane);
                    }}
                  >
                    <div className="jobs-lane__head">
                      <div className="jobs-lane__title">
                        <b>{lane.name}</b>
                        <span className="jobs-lane__count">{waiting.length} queued</span>
                      </div>
                      <span className="jobs-lane__state">
                        <StatusChip tone={ls.tone} symbol="" word={ls.word} />
                      </span>
                      <span className="jobs-lane__now">{ls.now}</span>
                      {lane.printer_id != null && (
                        <div className="jobs-lane__actions">
                          {!lane.plate_clear && st?.state !== "printing" && st?.state !== "paused" && (
                            <button
                              type="button"
                              className="jobs-btn jobs-btn--primary"
                              disabled={busy != null}
                              onClick={() => void plateCleared(lane)}
                            >
                              Plate cleared
                            </button>
                          )}
                          <button
                            type="button"
                            className="jobs-btn"
                            disabled={!canStart || busy != null}
                            title={canStart ? "Start this printer's next job now" : "Nothing to start, or a job is running"}
                            onClick={() => void startNext(lane)}
                          >
                            {busy === `start-${lane.printer_id}` ? "Starting…" : "Start next now"}
                          </button>
                        </div>
                      )}
                    </div>
                    <div className="jobs-lane__body">
                      {lane.items.length === 0 && (
                        <p className="jobs-lane__empty">
                          {lane.printer_id == null
                            ? "Nothing waiting."
                            : "Nothing queued here. Drop a card or let the fleet queue fill it."}
                        </p>
                      )}
                      {lane.items.map((c) => {
                        const movable = c.item.status === "waiting" || c.item.status === "failed";
                        const wi = waiting.findIndex((x) => x.item.id === c.item.id);
                        const showLine = isDrop && c.item.status === "waiting" && dropAt?.index === wi;
                        const meta = [
                          c.material,
                          fmtTime(c.print_time_secs),
                          c.grams != null ? `${Math.round(c.grams)} g` : null,
                        ]
                          .filter(Boolean)
                          .join(" · ");
                        return (
                          <div key={c.item.id}>
                            {showLine && <div className="jobs-drop-line" aria-hidden="true" />}
                            <article
                              className={`jobs-card${drag === c.item.id ? " jobs-card--dragging" : ""}`}
                              draggable={movable}
                              onDragStart={(e) => {
                                try {
                                  e.dataTransfer.setData("text/plain", String(c.item.id));
                                  e.dataTransfer.effectAllowed = "move";
                                } catch {
                                  /* some browsers refuse custom data */
                                }
                                setDrag(c.item.id);
                                setMenu(null);
                              }}
                              onDragEnd={() => {
                                setDrag(null);
                                setDropAt(null);
                              }}
                              onDragOver={(e) => {
                                if (drag == null) return;
                                e.preventDefault();
                                e.stopPropagation();
                                if (c.item.status !== "waiting") {
                                  if (dropAt?.lane !== key || dropAt.index !== 0) setDropAt({ lane: key, index: 0 });
                                  return;
                                }
                                const r = e.currentTarget.getBoundingClientRect();
                                const index = e.clientY < r.top + r.height / 2 ? wi : wi + 1;
                                if (dropAt?.lane !== key || dropAt.index !== index) setDropAt({ lane: key, index });
                              }}
                            >
                              <div className="jobs-card__top">
                                <span aria-hidden="true" className={`jobs-card__grip${movable ? "" : " jobs-card__grip--off"}`}>
                                  ⋮⋮
                                </span>
                                <Thumb path={c.thumb_url} colour={c.colour} />
                                <span className="jobs-card__file">{c.item.file_name}</span>
                                {c.item.status !== "printing" && (
                                  <button
                                    type="button"
                                    className="jobs-card__more"
                                    aria-label={`Actions for ${c.item.file_name}`}
                                    aria-expanded={menu === c.item.id}
                                    onClick={() => setMenu(menu === c.item.id ? null : c.item.id)}
                                  >
                                    ⋯
                                  </button>
                                )}
                              </div>
                              {meta && (
                                <div className="jobs-card__meta">
                                  <span
                                    aria-hidden="true"
                                    className="jobs-card__swatch"
                                    style={c.colour ? { background: c.colour } : undefined}
                                  />
                                  {meta}
                                </div>
                              )}
                              {c.item.source !== "manual" && (
                                <span className="jobs-card__source">from {c.item.source}</span>
                              )}
                              <span className="jobs-card__tag">
                                <StatusChip tone={c.tone as Tone} symbol="" word={c.tag} />
                              </span>
                              {menu === c.item.id && (
                                <div role="group" aria-label={`Actions for ${c.item.file_name}`} className="jobs-menu">
                                  {movable && (
                                    <>
                                      <button type="button" onClick={() => void doMove(c, c.item.printer_id, 0)}>
                                        {c.item.status === "failed" ? "Retry (move to top)" : "Move to top"}
                                      </button>
                                      <div className="jobs-menu__label">MOVE TO</div>
                                      {lanes
                                        .filter((l) => l.printer_id !== c.item.printer_id)
                                        .map((l) => (
                                          <button
                                            key={laneKey(l)}
                                            type="button"
                                            className="jobs-menu__sub"
                                            onClick={() =>
                                              void doMove(c, l.printer_id, l.items.filter((x) => x.item.status === "waiting").length)
                                            }
                                          >
                                            {l.name}
                                          </button>
                                        ))}
                                    </>
                                  )}
                                  <button type="button" className="jobs-menu__danger" onClick={() => void doRemove(c)}>
                                    {c.item.status === "sending" ? "Cancel upload" : "Remove from queue"}
                                  </button>
                                </div>
                              )}
                            </article>
                          </div>
                        );
                      })}
                      {isDrop && dropAt?.index === waiting.length && waiting.length > 0 && (
                        <div className="jobs-drop-line" aria-hidden="true" />
                      )}
                    </div>
                  </section>
                );
              })}
            </div>
          </div>
        </main>
      )}
      <ToastStack toasts={toasts} onDismiss={removeToast} />
    </div>
  );
}
