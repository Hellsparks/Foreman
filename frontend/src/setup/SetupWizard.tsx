/**
 * First-run setup (task 6.4, W1): filament data → printers → finish.
 * The server keeps the step, so leaving and coming back continues here.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { ApiError } from "../api/client";
import { setupApi, type DataSourceKind, type PendingImport, type SetupOverview } from "../api/setup";
import { Button } from "../ui";
import Checklist from "./Checklist";
import FoundPrinters from "./FoundPrinters";
import { countsLine, isEmpty, plural, revisionLine, STEPS, stepIndex } from "./setupLogic";
import "./setup.css";

function errText(e: unknown): string {
  return e instanceof ApiError || e instanceof Error ? e.message : String(e);
}

export default function SetupWizard() {
  const navigate = useNavigate();
  const [ov, setOv] = useState<SetupOverview | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      setOv(await setupApi.get());
      setErr(null);
    } catch (e) {
      setErr(errText(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const act = async (fn: () => Promise<SetupOverview | void>) => {
    setBusy(true);
    setErr(null);
    try {
      const r = await fn();
      if (r) setOv(r);
    } catch (e) {
      setErr(errText(e));
    } finally {
      setBusy(false);
    }
  };

  if (!ov) {
    return (
      <div className="su-page">
        <p className="su-muted">{err ?? "Loading setup…"}</p>
      </div>
    );
  }

  const step = ov.state.step;
  const idx = stepIndex(step);

  return (
    <div className="su-page">
      <header className="su-head">
        <div>
          <p className="page-header__eyebrow">First-run setup</p>
          <h1 className="su-title phone-dup-title">Set up Foreman</h1>
        </div>
        {step !== "done" && (
          <Button
            variant="secondary"
            disabled={busy}
            onClick={() =>
              void act(async () => {
                await setupApi.skip();
                navigate("/fleet");
              })
            }
          >
            Skip setup
          </Button>
        )}
      </header>

      <ol className="su-steps" aria-label="Setup steps">
        {STEPS.map((s, i) => (
          <li
            key={s.step}
            className={`su-steps__item${i === idx ? " su-steps__item--now" : ""}${i < idx ? " su-steps__item--done" : ""}`}
            aria-current={i === idx ? "step" : undefined}
          >
            <span className="su-steps__n">{i < idx ? "✓" : i + 1}</span> {s.label}
          </li>
        ))}
      </ol>

      {err && (
        <p className="su-error" role="alert">
          {err}
        </p>
      )}

      <div className="su-body page-fill">
        {idx < STEPS.length && (
          <p className="su-phone-only su-eyebrow">
            Step {idx + 1} of {STEPS.length} · {STEPS[idx].label}
          </p>
        )}
        {step === "data" && <DataStep ov={ov} busy={busy} act={act} />}
        {step === "printers" && <PrintersStep ov={ov} busy={busy} act={act} reload={load} />}
        {step === "finish" && <FinishStep ov={ov} busy={busy} act={act} reload={load} />}
        {step === "done" && (
          <section className="su-card">
            <h2>Setup is done</h2>
            <p className="su-muted">Run it again from Settings whenever you like.</p>
            <div className="su-actions">
              <Button variant="primary" onClick={() => navigate("/fleet")}>
                Go to Fleet
              </Button>
              <Button variant="secondary" disabled={busy} onClick={() => void act(() => setupApi.restart())}>
                Run setup again
              </Button>
            </div>
          </section>
        )}
        {step !== "done" && (
          <div className="su-phone-only su-actions">
            <Button
              variant="secondary"
              disabled={busy}
              onClick={() =>
                void act(async () => {
                  await setupApi.skip();
                  navigate("/fleet");
                })
              }
            >
              Skip setup
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}

type Act = (fn: () => Promise<SetupOverview | void>) => Promise<void>;

// ---------------------------------------------------------------------------
// Step 1: filament data
// ---------------------------------------------------------------------------

function DataStep({ ov, busy, act }: { ov: SetupOverview; busy: boolean; act: Act }) {
  const [choice, setChoice] = useState<DataSourceKind | null>(
    ov.state.pending?.kind ?? ov.state.data_source?.kind ?? null,
  );
  const [pending, setPending] = useState<PendingImport | null>(ov.state.pending);
  const [path, setPath] = useState("");
  const [url, setUrl] = useState("");
  const [replace, setReplace] = useState(false);
  const [detecting, setDetecting] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const cur = ov.current;
  const hasData = !isEmpty(cur);
  const smOff = !ov.spoolman_running;

  const detect = async (fn: () => Promise<PendingImport>) => {
    setDetecting(true);
    setErr(null);
    try {
      setPending(await fn());
    } catch (e) {
      setErr(errText(e));
      setPending(null);
    } finally {
      setDetecting(false);
    }
  };

  const kindPending = pending && pending.kind === choice ? pending : null;
  const port = ov.spoolman_port ?? 7912;

  const options: { kind: DataSourceKind; title: string; means: string; more: string }[] = [
    {
      kind: "fresh",
      title: "Fresh start",
      means: hasData
        ? `Keeps what Foreman holds now: ${countsLine(cur)}.`
        : "An empty filament inventory. Add spools under Filament later.",
      more: "Nothing is copied from anywhere.",
    },
    {
      kind: "spoolman_file",
      title: "Use an existing Spoolman database",
      means: hasData
        ? `Copies the file's vendors, filaments and spools in, replacing the ${plural(cur.spools, "spool", "spools")} Foreman holds now. The file is not changed.`
        : "Copies the file's vendors, filaments, spools and custom fields in, with the same ids. The file is not changed.",
      more: "The spoolman.db file from Spoolman's data folder (~/.local/share/spoolman on Linux) or one of its backups.",
    },
    {
      kind: "spoolman_url",
      title: "Connect to a running Spoolman",
      means: hasData
        ? `Reads that Spoolman once and copies its data in, replacing the ${plural(cur.spools, "spool", "spools")} Foreman holds now. That Spoolman is not changed.`
        : "Reads that Spoolman once and copies its data in, with the same ids. That Spoolman is not changed.",
      more: `Afterwards point Moonraker's [spoolman] server at Foreman (port ${port}) so filament use lands here, then retire the old Spoolman.`,
    },
  ];

  return (
    <section className="su-card" aria-labelledby="su-data-h">
      <h2 id="su-data-h">Where does filament data come from?</h2>
      {smOff && (
        <p className="su-note" role="note">
          The built-in Spoolman is off on this server (FOREMAN__SPOOLMAN_ENABLED=false), so only a fresh start is possible.
        </p>
      )}
      <div className="su-choices" role="radiogroup" aria-label="Filament data source">
        {options.map((o) => {
          const disabled = smOff && o.kind !== "fresh";
          return (
            <label key={o.kind} className={`su-choice${choice === o.kind ? " su-choice--on" : ""}`}>
              <input
                type="radio"
                name="su-source"
                className="su-check"
                checked={choice === o.kind}
                disabled={disabled || busy}
                onChange={() => {
                  setChoice(o.kind);
                  setErr(null);
                }}
              />
              <span className="su-choice__text">
                <b>{o.title}</b>
                <span className="su-choice__means">{o.means}</span>
                <span className="su-muted su-more">{o.more}</span>
              </span>
            </label>
          );
        })}
      </div>

      {choice === "spoolman_file" && (
        <div className="su-panel">
          <div className="su-row__fields">
            <label className="su-field">
              <span>Upload the file</span>
              <input
                ref={fileRef}
                type="file"
                accept=".db,.sqlite,.sqlite3,application/octet-stream"
                className="su-file"
                disabled={detecting}
                onChange={(e) => {
                  const f = e.target.files?.[0];
                  if (f) void detect(() => setupApi.detectUpload(f));
                }}
              />
            </label>
            <label className="su-field">
              <span>…or its path on the server</span>
              <span className="su-inline">
                <input
                  className="auth__input auth__mono"
                  placeholder="/home/pi/.local/share/spoolman/spoolman.db"
                  value={path}
                  onChange={(e) => setPath(e.target.value)}
                />
                <Button variant="secondary" disabled={detecting || !path.trim()} onClick={() => void detect(() => setupApi.detectPath(path.trim()))}>
                  Detect
                </Button>
              </span>
            </label>
          </div>
        </div>
      )}

      {choice === "spoolman_url" && (
        <div className="su-panel">
          <label className="su-field">
            <span>Spoolman address</span>
            <span className="su-inline">
              <input
                className="auth__input auth__mono"
                placeholder="http://192.168.1.20:7912"
                value={url}
                onChange={(e) => setUrl(e.target.value)}
              />
              <Button variant="secondary" disabled={detecting || !url.trim()} onClick={() => void detect(() => setupApi.detectUrl(url.trim()))}>
                Read
              </Button>
            </span>
          </label>
        </div>
      )}

      {detecting && <p className="su-muted">Reading…</p>}
      {err && (
        <p className="su-error" role="alert">
          {err}
        </p>
      )}

      {kindPending && (
        <div className={`su-detect${kindPending.accepted ? "" : " su-detect--refused"}`} aria-label="What was found">
          <div className="su-kv">
            <span>Source</span>
            <span className="su-trunc">{kindPending.label}</span>
          </div>
          <div className="su-kv">
            <span>Engine</span>
            <span>{kindPending.engine}</span>
          </div>
          <div className="su-kv">
            <span>Revision</span>
            <span>{revisionLine(kindPending)}</span>
          </div>
          <div className="su-kv">
            <span>Contains</span>
            <span>
              {countsLine(kindPending.counts)}
              {kindPending.counts.extra_fields > 0 ? ` · ${plural(kindPending.counts.extra_fields, "custom field", "custom fields")}` : ""}
            </span>
          </div>
          {kindPending.refusal ? (
            <p className="bk-danger" role="alert">
              {kindPending.refusal}
            </p>
          ) : (
            hasData && (
              <label className="su-toggle">
                <input type="checkbox" className="su-check" checked={replace} onChange={(e) => setReplace(e.target.checked)} />
                <span>Replace the {countsLine(cur)} in Foreman now</span>
              </label>
            )
          )}
        </div>
      )}

      <div className="su-actions su-actions--end">
        {choice === "fresh" && (
          <Button variant="primary" disabled={busy} onClick={() => void act(() => setupApi.fresh())}>
            Continue with a fresh start
          </Button>
        )}
        {kindPending && kindPending.accepted && (
          <Button
            variant="primary"
            disabled={busy || (hasData && !replace)}
            onClick={() => void act(() => setupApi.importData(replace))}
          >
            {busy ? "Importing…" : `Import ${plural(kindPending.counts.spools, "spool", "spools")}`}
          </Button>
        )}
        {kindPending && (
          <Button
            variant="secondary"
            disabled={busy}
            onClick={() =>
              void act(async () => {
                const r = await setupApi.discard();
                setPending(null);
                if (fileRef.current) fileRef.current.value = "";
                return r;
              })
            }
          >
            Discard
          </Button>
        )}
        {ov.state.data_source && !kindPending && choice !== "fresh" && (
          <Button variant="secondary" disabled={busy} onClick={() => void act(() => setupApi.step("printers"))}>
            Keep current data ({ov.state.data_source.label})
          </Button>
        )}
      </div>
    </section>
  );
}

// ---------------------------------------------------------------------------
// Step 2: printers
// ---------------------------------------------------------------------------

function PrintersStep({ ov, busy, act, reload }: { ov: SetupOverview; busy: boolean; act: Act; reload: () => Promise<void> }) {
  const printers = ov.checklist.items.find((i) => i.id === "printers");
  const added = !!printers?.done;
  return (
    <>
      <FoundPrinters onAdded={() => void reload()} />
      {added && (
        <p className="auth__hint auth__hint--ok" role="status">
          ✓ {printers?.title}: {printers?.detail}
        </p>
      )}
      <div className="su-actions su-actions--end">
        <Button variant="secondary" disabled={busy} onClick={() => void act(() => setupApi.step("data"))}>
          Back
        </Button>
        <Button variant={added ? "primary" : "secondary"} disabled={busy} onClick={() => void act(() => setupApi.step("finish"))}>
          {added ? "Continue" : "Skip for now"}
        </Button>
      </div>
    </>
  );
}

// ---------------------------------------------------------------------------
// Step 3: finish
// ---------------------------------------------------------------------------

function FinishStep({ ov, busy, act, reload }: { ov: SetupOverview; busy: boolean; act: Act; reload: () => Promise<void> }) {
  const navigate = useNavigate();
  const backupDone = !!ov.checklist.items.find((i) => i.id === "backup")?.done;
  // The 6.2 default schedule is offered on by default.
  const [backupOn, setBackupOn] = useState(true);
  const [dir, setDir] = useState(ov.backup_default_dir);

  const finish = () =>
    act(async () => {
      if (!backupDone && backupOn) await setupApi.backup(true, dir.trim());
      await setupApi.finish();
      navigate("/fleet");
    });

  return (
    <>
      <Checklist
        ov={ov}
        mode="wizard"
        onChange={() => void reload()}
        backup={{ enabled: backupOn, onEnabled: setBackupOn, dir, onDir: setDir }}
      />
      <div className="su-actions su-actions--end">
        <Button variant="secondary" disabled={busy} onClick={() => void act(() => setupApi.step("printers"))}>
          Back
        </Button>
        <Button variant="primary" disabled={busy || (!backupDone && backupOn && !dir.trim())} onClick={() => void finish()}>
          {busy ? "Finishing…" : !backupDone && backupOn ? "Turn on backups and finish" : "Finish"}
        </Button>
      </div>
    </>
  );
}
