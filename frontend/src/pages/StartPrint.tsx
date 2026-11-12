/**
 * Start print (doc 06 §5): the middle tab of the phone bottom bar.
 * Pick file → pick printer → confirm plate clear → go. Same checks and send
 * call as Library → Send (`/api/send/check`, `/api/send` print_now).
 */
import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { lib, send, type CompatResult, type LibraryEntry } from "../api/library";
import { hours, isManualSwaps } from "../api/projects";
import Thumb from "../projects/Thumb";
import { HeaderSearch, HeaderActions, PhoneTitle } from "../ui";
import "./StartPrint.css";

const PRINTABLE = ["gcode", "bgcode", "3mf"];

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

function verdictTone(v: string): string {
  return v === "ready" ? "var(--fw-success)" : v === "plate" ? "var(--fw-warning)" : v === "busy" || v === "offline" ? "var(--fw-muted)" : "var(--fw-danger)";
}

export default function StartPrint() {
  const [path, setPath] = useState("");
  const [rows, setRows] = useState<LibraryEntry[] | null>(null);
  const [q, setQ] = useState("");
  const [file, setFile] = useState<LibraryEntry | null>(null);
  const [compat, setCompat] = useState<CompatResult[] | null>(null);
  const [printer, setPrinter] = useState<CompatResult | null>(null);
  const [plateOk, setPlateOk] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [done, setDone] = useState("");
  const [swapAsk, setSwapAsk] = useState("");

  useEffect(() => {
    setRows(null);
    lib.list(path).then(
      (r) => setRows(r.entries),
      (e) => setError(errText(e)),
    );
  }, [path]);

  useEffect(() => {
    if (!file) return;
    setCompat(null);
    send.check(file.id).then(setCompat, (e) => setError(errText(e)));
  }, [file]);

  const shown = useMemo(() => {
    const words = q.toLowerCase().split(/\s+/).filter(Boolean);
    return (rows ?? []).filter(
      (e) => (e.kind === "folder" || PRINTABLE.includes(e.format ?? "")) && words.every((w) => e.name.toLowerCase().includes(w)),
    );
  }, [rows, q]);

  const step = !file ? 1 : !printer ? 2 : 3;
  const folderName = path ? path.split("/").pop() : "Library";

  const go = async (acceptSwaps = false) => {
    if (!file || !printer) return;
    setBusy(true);
    setError("");
    setSwapAsk("");
    try {
      const r = await send.send({
        file_id: file.id,
        printer_id: Number(printer.printer_id),
        mode: "print_now",
        plate_confirm: plateOk,
        accept_manual_swaps: acceptSwaps,
      });
      setDone(r.message);
    } catch (e) {
      if (isManualSwaps(e)) setSwapAsk(e.message);
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const restart = () => {
    setFile(null);
    setPrinter(null);
    setPlateOk(false);
    setDone("");
    setError("");
    setSwapAsk("");
  };

  return (
    <div className="sp-page">
      <h1 className="phone-dup-title">Start print</h1>
      <PhoneTitle>
        Start print <span>· {step === 1 ? "file" : step === 2 ? "printer" : "go"}</span>
      </PhoneTitle>
      {step === 1 && (
        <HeaderActions>
          <HeaderSearch value={q} onChange={setQ} label="Search this folder…" />
        </HeaderActions>
      )}

      {done ? (
        <div className="sp-done" role="status">
          <p className="sp-done__msg">✓ {done}</p>
          <div className="sp-actions">
            <Link to="/fleet" className="sp-btn sp-btn--primary">
              Go to Fleet
            </Link>
            <button type="button" className="sp-btn" onClick={restart}>
              Start another
            </button>
          </div>
        </div>
      ) : step === 1 ? (
        <>
          {path && (
            <button type="button" className="sp-back" onClick={() => setPath(path.split("/").slice(0, -1).join("/"))}>
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
                <path d="M15 5l-7 7 7 7" />
              </svg>
              {folderName}
            </button>
          )}
          {rows == null && !error && <p className="sp-note">Loading…</p>}
          {rows && shown.length === 0 && <p className="sp-note">{q ? "No match in this folder." : "No printable files here."}</p>}
          <ul className="sp-list" role="list">
            {shown.map((e) => (
              <li key={e.id}>
                <button type="button" className="sp-row" onClick={() => (e.kind === "folder" ? setPath(e.path) : setFile(e))}>
                  {e.kind === "folder" ? (
                    <span className="sp-row__icon" aria-hidden="true">
                      ▸
                    </span>
                  ) : (
                    <Thumb url={e.thumb_url} />
                  )}
                  <span className="sp-row__name">{e.name}</span>
                  <span className="sp-row__meta">{e.kind === "folder" ? "Folder" : e.print_time_secs ? hours(e.print_time_secs) : (e.format ?? "").toUpperCase()}</span>
                </button>
              </li>
            ))}
          </ul>
        </>
      ) : (
        <>
          <button type="button" className="sp-back" onClick={() => (step === 3 ? setPrinter(null) : setFile(null))}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
              <path d="M15 5l-7 7 7 7" />
            </svg>
            <span className="sp-back__name">{step === 3 && printer ? `${file?.name} → ${printer.printer_name}` : file?.name}</span>
          </button>
          {step === 2 && (
            <>
              {compat == null && !error && <p className="sp-note">Checking printers…</p>}
              {compat && compat.length === 0 && <p className="sp-note">No printers configured.</p>}
              <ul className="sp-list" role="list">
                {(compat ?? []).map((c) => {
                  const can = c.verdict === "ready" || c.verdict === "plate";
                  return (
                    <li key={Number(c.printer_id)}>
                      <button type="button" className="sp-row" disabled={!can} onClick={() => setPrinter(c)}>
                        <span className="sp-row__icon" style={{ color: verdictTone(c.verdict) }} aria-hidden="true">
                          ●
                        </span>
                        <span className="sp-row__name">{c.printer_name}</span>
                        <span className="sp-row__meta">{c.reason}</span>
                      </button>
                    </li>
                  );
                })}
              </ul>
            </>
          )}
          {step === 3 && printer && (
            <div className="sp-confirm">
              {printer.verdict === "plate" && (
                <label className="sp-check">
                  <input type="checkbox" checked={plateOk} onChange={(e) => setPlateOk(e.target.checked)} />
                  Plate is clear — I removed the last print
                </label>
              )}
              {swapAsk && (
                <button type="button" className="sp-btn" disabled={busy} onClick={() => void go(true)}>
                  Swap filament by hand · print anyway
                </button>
              )}
              <button
                type="button"
                className="sp-btn sp-btn--primary"
                disabled={busy || (printer.verdict === "plate" && !plateOk)}
                onClick={() => void go()}
              >
                {busy ? "Starting…" : `Print on ${printer.printer_name}`}
              </button>
            </div>
          )}
        </>
      )}
      {error && !done && (
        <p className="sp-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
