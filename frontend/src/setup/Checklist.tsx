/**
 * Getting-started checklist (FleetEmpty.dc.html): the wizard's finish step and the
 * empty Fleet page. Rows: server, filament source, printers, slicer, backups.
 */
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { ApiError } from "../api/client";
import { setupApi, type ChecklistItem, type SetupOverview, type SlicerKeyResult } from "../api/setup";
import { Button } from "../ui";
import { checklistHeading } from "./setupLogic";

function errText(e: unknown): string {
  return e instanceof ApiError || e instanceof Error ? e.message : String(e);
}

/** The secret.key warning from Settings → Backup (6.2), short form. */
export function SecretKeyWarning() {
  return (
    <div className="bk-warn su-warn" role="note">
      <b>Backups contain secret.key.</b> It unlocks the SSH key and printer passwords stored in Foreman. Keep backup files
      somewhere private, or turn on encryption in Settings → Backup.
    </div>
  );
}

/** "Back up daily at 03:00 · keep 7" + folder. Controlled, so the wizard can apply it on Finish. */
export function BackupOffer({
  enabled,
  onEnabled,
  dir,
  onDir,
}: {
  enabled: boolean;
  onEnabled: (v: boolean) => void;
  dir: string;
  onDir: (v: string) => void;
}) {
  return (
    <div className="su-panel">
      <label className="su-toggle">
        <input type="checkbox" className="su-check" checked={enabled} onChange={(e) => onEnabled(e.target.checked)} />
        <span>Back up daily at 03:00, keep the last 7</span>
      </label>
      <label className="su-field">
        <span>Folder on this machine</span>
        <input className="auth__input auth__mono" value={dir} onChange={(e) => onDir(e.target.value)} />
      </label>
      <SecretKeyWarning />
    </div>
  );
}

/** Slicer URL + one-click API key (secret shown once). */
export function SlicerPanel({ ov, onChange }: { ov: SetupOverview; onChange: () => void }) {
  const [made, setMade] = useState<SlicerKeyResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [copied, setCopied] = useState<string | null>(null);
  const urls = made?.urls ?? ov.checklist.slicer.urls;
  const copy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(text);
    } catch {
      setCopied(null);
    }
  };
  const generate = async () => {
    setBusy(true);
    setErr(null);
    try {
      setMade(await setupApi.slicerKey());
      onChange();
    } catch (e) {
      setErr(errText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="su-panel">
      <p className="su-muted su-more">
        In OrcaSlicer or PrusaSlicer: printer settings → Physical printer → host type <b>OctoPrint</b>. Use this URL and an API
        key; uploads land in Library → Slicer inbox.
      </p>
      <div className="su-kv">
        <span>URL</span>
        <div className="su-urls">
          {urls.map((u, i) => (
            <span key={u} className="su-copyrow">
              <code className="su-code">{u}</code>
              {i === 0 && (
                <Button variant="small" onClick={() => void copy(u)}>
                  {copied === u ? "Copied" : "Copy"}
                </Button>
              )}
            </span>
          ))}
        </div>
      </div>
      <div className="su-kv">
        <span>API key</span>
        {made ? (
          <div className="su-secret">
            <span className="su-copyrow">
              <code className="su-code su-code--secret">{made.secret}</code>
              <Button variant="small" onClick={() => void copy(made.secret)}>
                {copied === made.secret ? "Copied" : "Copy"}
              </Button>
            </span>
            <p className="su-warn-line" role="note">
              Shown once. Paste it into the slicer now; Foreman keeps only a hash.
            </p>
          </div>
        ) : ov.checklist.slicer.setup_key_exists ? (
          <p className="su-muted">
            The slicer key was made already and shown once. Revoke it in Settings → Security to make a new one.
          </p>
        ) : (
          <Button variant="primary" disabled={busy} onClick={() => void generate()}>
            {busy ? "Generating…" : "Generate API key"}
          </Button>
        )}
      </div>
      {err && (
        <p className="su-error" role="alert">
          {err}
        </p>
      )}
    </div>
  );
}

export default function Checklist({
  ov,
  onChange,
  mode,
  backup,
}: {
  ov: SetupOverview;
  onChange: () => void;
  /** `fleet`: rows carry their own actions. `wizard`: the finish step applies the backup offer. */
  mode: "fleet" | "wizard";
  /** Wizard only: the backup offer's state, applied by the Finish button. */
  backup?: { enabled: boolean; onEnabled: (v: boolean) => void; dir: string; onDir: (v: string) => void };
}) {
  const navigate = useNavigate();
  const [open, setOpen] = useState<string | null>(mode === "wizard" ? "backup" : null);
  const [fleetBackupOn, setFleetBackupOn] = useState(true);
  const [fleetDir, setFleetDir] = useState(ov.backup_default_dir);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const items = ov.checklist.items;

  const toggle = (id: string) => setOpen((o) => (o === id ? null : id));

  const goWizard = async (step: "data" | "printers") => {
    setErr(null);
    try {
      if (!ov.needs_wizard) await setupApi.restart();
      if (step === "printers" && ov.state.data_source) await setupApi.step("printers");
      navigate("/setup");
    } catch (e) {
      setErr(errText(e));
    }
  };

  const turnOnBackups = async () => {
    setBusy(true);
    setErr(null);
    try {
      await setupApi.backup(fleetBackupOn, fleetDir.trim());
      setOpen(null);
      onChange();
    } catch (e) {
      setErr(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const action = (it: ChecklistItem) => {
    switch (it.id) {
      case "filament":
        if (it.done && mode === "fleet") return null;
        return (
          <Button variant="small" onClick={() => void goWizard("data")}>
            {it.done ? "Change" : "Choose"}
          </Button>
        );
      case "printers":
        return mode === "wizard" ? (
          <Button variant="small" onClick={() => void goWizard("printers")}>
            {it.done ? "Add more" : "Add printers"}
          </Button>
        ) : null;
      case "slicer":
        return (
          <Button variant="small" aria-expanded={open === "slicer"} onClick={() => toggle("slicer")}>
            <span className="su-long">{open === "slicer" ? "Hide slicer settings" : "Show slicer settings"}</span>
            <span className="su-short" aria-hidden="true">
              Slicer
            </span>
          </Button>
        );
      case "backup":
        if (it.done || mode === "wizard") return null;
        return (
          <Button variant="small" aria-expanded={open === "backup"} onClick={() => toggle("backup")}>
            {open === "backup" ? "Close" : "Turn on"}
          </Button>
        );
      default:
        return null;
    }
  };

  return (
    <section className="su-list" aria-label="Getting started">
      <div className="su-list__cap">{checklistHeading(ov.checklist.done, ov.checklist.total)}</div>
      {items.map((it) => (
        <div key={it.id} className="su-item">
          <div className={`su-item__row${it.done ? " su-item__row--done" : ""}`}>
            <span className={`su-item__mark ${it.done ? "su-item__mark--done" : "su-item__mark--todo"}`}>
              {it.done ? "✓" : "○"}
              <span className="su-item__word"> {it.done ? "Done" : "To do"}</span>
            </span>
            <span className="su-item__text" title={it.detail ? `${it.title} · ${it.detail}` : it.title}>
              <span className="su-item__title">{it.title}</span>
              {it.detail && <span className="su-item__detail"> · {it.detail}</span>}
            </span>
            {action(it)}
          </div>
          {it.id === "slicer" && open === "slicer" && <SlicerPanel ov={ov} onChange={onChange} />}
          {it.id === "backup" && mode === "wizard" && !it.done && backup && <BackupOffer {...backup} />}
          {it.id === "backup" && mode === "fleet" && open === "backup" && (
            <div>
              <BackupOffer enabled={fleetBackupOn} onEnabled={setFleetBackupOn} dir={fleetDir} onDir={setFleetDir} />
              <div className="su-actions su-actions--inset">
                <Button variant="primary" disabled={busy || !fleetBackupOn || !fleetDir.trim()} onClick={() => void turnOnBackups()}>
                  {busy ? "Saving…" : "Turn on backups"}
                </Button>
              </div>
            </div>
          )}
        </div>
      ))}
      {err && (
        <p className="su-error su-error--inset" role="alert">
          {err}
        </p>
      )}
    </section>
  );
}
