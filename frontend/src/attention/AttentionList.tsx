/**
 * Open Attention items with their actions (§3, doc 06): the bell panel and the
 * Attention page render this. Moved out of the Fleet page, which is printers only.
 * Card per item after PhoneFleet.dc.html: chip, sentence, time, actions in one row.
 */
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { api, type AttentionItem } from "../api/client";
import { FAIL_REASONS, jobs, type JobOutcome } from "../api/jobs";
import { queue } from "../api/queue";
import { StatusChip } from "../ui";
import { relTime } from "../ui/relTime";
import { attentionSymbol, attentionTone, attentionWord, isService, serviceTask } from "./attentionLogic";
import "./Attention.css";

interface Props {
  items: AttentionItem[];
  /** Printer names by id, for the meta line. */
  printerNames?: Record<number, string>;
  /** An item was resolved here (drop it from the list). */
  onResolved: (id: number) => void;
  /** Called before navigating away (the bell closes its panel). */
  onNavigate?: () => void;
}

interface Action {
  label: string;
  run: () => void;
  primary?: boolean;
}

export default function AttentionList({ items, printerNames = {}, onResolved, onNavigate }: Props) {
  const navigate = useNavigate();
  const [rated, setRated] = useState<Record<number, { outcome: JobOutcome; reason?: string }>>({});
  const [reasonFor, setReasonFor] = useState<number | null>(null);
  const [error, setError] = useState("");

  const go = (to: string) => {
    onNavigate?.();
    navigate(to);
  };

  const attempt = async (f: () => Promise<unknown>, id: number) => {
    setError("");
    try {
      await f();
      onResolved(id);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const rate = async (item: AttentionItem, outcome: JobOutcome, reason?: string) => {
    if (item.job_id == null) return;
    try {
      await jobs.rate(item.job_id, outcome, reason ?? null);
      setRated((prev) => ({ ...prev, [item.id]: { outcome, reason } }));
      setReasonFor(outcome === "failed" && !reason ? item.id : null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const actionsFor = (item: AttentionItem): Action[] => {
    const acts: Action[] = [];
    if (item.kind === "print_finished" && item.printer_id != null) {
      const pid = item.printer_id;
      acts.push({ label: "Plate cleared", primary: true, run: () => void attempt(() => queue.plateCleared(pid), item.id) });
    }
    if (isService(item.kind)) {
      acts.push({ label: "Open in Service", primary: true, run: () => go(`/service?task=${serviceTask(item.kind)}`) });
    }
    if (item.printer_id != null) {
      const pid = item.printer_id;
      acts.push({ label: "Open printer", run: () => go(`/printer/${pid}`) });
    }
    // A service item clears when the task is marked done.
    if (!isService(item.kind)) {
      acts.push({ label: "Dismiss", run: () => void attempt(() => api.dismissAttention(item.id), item.id) });
    }
    return acts;
  };

  if (items.length === 0) {
    return <p className="attn-empty">✓ Nothing needs you right now.</p>;
  }

  return (
    <>
      {error && (
        <p className="attn-error" role="alert">
          {error}
        </p>
      )}
      <ul className="attn-list" role="list">
        {items.map((item) => {
          const r = rated[item.id];
          const printer = item.printer_id != null ? printerNames[item.printer_id] : undefined;
          const meta = [relTime(item.created_at), printer].filter(Boolean).join(" · ");
          return (
            <li key={item.id} className="attn-card">
              <div className="attn-card__head">
                <StatusChip tone={attentionTone(item.kind)} symbol={attentionSymbol(item.kind)} word={attentionWord(item.kind)} />
                <div className="attn-card__text">
                  <div className="attn-card__title">{item.title}</div>
                  {item.body && <div className="attn-card__body">{item.body}</div>}
                  {meta && <div className="attn-card__meta">{meta}</div>}
                </div>
              </div>
              {item.kind === "print_finished" && item.job_id != null && (
                <div className="attn-rate" role="group" aria-label="How did it go?">
                  <span className="attn-rate__label">
                    {r ? (r.outcome === "good" ? "Rated ✓ Good" : `Rated ✖ Failed${r.reason ? ` · ${r.reason}` : ""}`) : "How did it go?"}
                  </span>
                  <button type="button" className="attn-chip" aria-pressed={r?.outcome === "good"} onClick={() => void rate(item, "good")}>
                    ✓ Good
                  </button>
                  <button type="button" className="attn-chip" aria-pressed={r?.outcome === "failed"} onClick={() => void rate(item, "failed")}>
                    ✖ Failed
                  </button>
                  {reasonFor === item.id &&
                    FAIL_REASONS.map((reason) => (
                      <button key={reason} type="button" className="attn-chip attn-chip--reason" onClick={() => void rate(item, "failed", reason)}>
                        {reason}
                      </button>
                    ))}
                </div>
              )}
              <div className="attn-card__actions">
                {actionsFor(item).map((a) => (
                  <button key={a.label} type="button" className={`attn-btn${a.primary ? " attn-btn--primary" : ""}`} onClick={a.run}>
                    {a.label}
                  </button>
                ))}
              </div>
            </li>
          );
        })}
      </ul>
    </>
  );
}
