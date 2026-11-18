/** Printer → History: the printer's Bambu Cloud print history, read-only (doc 07 §B). */
import { useEffect, useState } from "react";
import { bambuApi, type CloudHistory, type CloudTask } from "../api/bambu";
import { StatusChip, type Tone } from "../ui";
import { fmtDuration, fmtGrams } from "../pages/JobsHistory";
import "./BambuCloud.css";

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export function taskChip(t: CloudTask): { tone: Tone; symbol: string; word: string } {
  switch (t.outcome) {
    case "printing":
      return { tone: "info", symbol: "◷", word: "Printing" };
    case "finished":
      return { tone: "ok", symbol: "●", word: "Finished" };
    case "failed":
      return { tone: "bad", symbol: "✖", word: "Failed / cancelled" };
    default:
      return { tone: "off", symbol: "○", word: t.status != null ? `Status ${t.status}` : "Unknown" };
  }
}

/** Wall-clock seconds between start and end (Bambu's own `costTime` is the slice estimate). */
export function taskSeconds(t: CloudTask): number | null {
  if (!t.start_time || !t.end_time) return null;
  const s = (Date.parse(t.end_time) - Date.parse(t.start_time)) / 1000;
  return Number.isFinite(s) && s >= 0 ? s : null;
}

export function CloudHistoryPanel({ printerId }: { printerId: number }) {
  const [data, setData] = useState<CloudHistory | null>(null);
  const [err, setErr] = useState("");

  useEffect(() => {
    let live = true;
    bambuApi
      .history(printerId)
      .then((d) => live && setData(d))
      .catch((e) => live && setErr(errText(e)));
    return () => {
      live = false;
    };
  }, [printerId]);

  return (
    <section className="bc__history" aria-label="Bambu Cloud print history">
      <span className="auth__cap">
        Bambu Cloud history{data ? ` · ${data.account_label}` : ""} · read-only
      </span>
      {err && <p className="auth__hint bc__err" role="alert">{err}</p>}
      {!err && !data && <p className="auth__hint">Loading…</p>}
      {data && data.tasks.length === 0 && <p className="auth__hint">No cloud prints for this printer.</p>}
      {data && data.tasks.length > 0 && (
        <table className="bc__table">
          <thead>
            <tr>
              <th scope="col">Print</th>
              <th scope="col">Started</th>
              <th scope="col">Took</th>
              <th scope="col">Planned</th>
              <th scope="col">Result</th>
            </tr>
          </thead>
          <tbody>
            {data.tasks.map((t) => {
              const c = taskChip(t);
              return (
                <tr key={t.id}>
                  <td>
                    {t.title || "—"}
                    {t.plate_name ? ` · ${t.plate_name}` : ""}
                  </td>
                  <td>{t.start_time ? new Date(t.start_time).toLocaleString() : "—"}</td>
                  <td>{fmtDuration(taskSeconds(t))}</td>
                  <td>{fmtGrams(t.weight_g)}</td>
                  <td>
                    <StatusChip tone={c.tone} symbol={c.symbol} word={c.word} />
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </section>
  );
}
