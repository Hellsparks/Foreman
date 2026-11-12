/**
 * Settings → Backup (task 6.2, W14): folder, schedule, retention, Library opt-in,
 * passphrase encryption, Run now, and restore (check → confirm → restart).
 */
import { useCallback, useEffect, useState } from "react";
import { backupApi, type BackupEntry, type BackupOverview, type BackupSettingsDto, type InspectResult } from "../api/backup";
import Button from "../ui/Button";
import { compareRows, countsLine, fmtBytes, fmtWhen, scheduleText, WEEKDAYS } from "./backupLogic";
import "./Backup.css";

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

const cap = { margin: "var(--fw-space-6) 0 var(--fw-space-2)", fontWeight: "normal" } as const;

export default function BackupSettings() {
  const [ov, setOv] = useState<BackupOverview | null>(null);
  const [draft, setDraft] = useState<BackupSettingsDto | null>(null);
  const [pass, setPass] = useState("");
  const [error, setError] = useState("");
  const [note, setNote] = useState("");
  const [dirCheck, setDirCheck] = useState<{ ok: boolean; message: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [target, setTarget] = useState<string | null>(null);
  const [otherPath, setOtherPath] = useState("");

  const load = useCallback(async () => {
    try {
      const o = await backupApi.overview();
      setOv(o);
      setDraft(o.settings);
      setError("");
    } catch (e) {
      setError(errText(e));
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load]);

  if (!ov || !draft) {
    return (
      <section aria-labelledby="backup-heading" style={{ maxWidth: 720, marginTop: "var(--fw-space-8)" }}>
        <h2 id="backup-heading" className="bk-h2" style={{ font: "500 18px/1.3 var(--fw-sans)", margin: "0 0 var(--fw-space-4)" }}>
          Backup
        </h2>
        <p className="auth__hint">{error || "Loading…"}</p>
      </section>
    );
  }

  const set = (p: Partial<BackupSettingsDto>) => setDraft({ ...draft, ...p });
  const dirty = JSON.stringify(draft) !== JSON.stringify(ov.settings) || pass !== "";

  async function act(f: () => Promise<void>) {
    setBusy(true);
    setError("");
    setNote("");
    try {
      await f();
    } catch (e) {
      setError(errText(e));
      void load();
    } finally {
      setBusy(false);
    }
  }

  const save = () =>
    act(async () => {
      const o = await backupApi.save(draft, pass === "" ? undefined : pass);
      setOv(o);
      setDraft(o.settings);
      setPass("");
      setNote("Saved.");
    });
  const runNow = () =>
    act(async () => {
      const r = await backupApi.run();
      setNote(`Backup written: ${r.file ?? ""} (${fmtBytes(r.size ?? 0)})${r.pruned.length ? ` · removed ${r.pruned.length} old` : ""}.`);
      await load();
    });
  const checkDir = () =>
    act(async () => {
      setDirCheck(await backupApi.checkDir(draft.dir));
    });

  const last = ov.last;

  return (
    <section aria-labelledby="backup-heading" style={{ maxWidth: 720, marginTop: "var(--fw-space-8)" }}>
      <h2 id="backup-heading" style={{ font: "500 18px/1.3 var(--fw-sans)", margin: "0 0 var(--fw-space-4)", letterSpacing: "-0.02em" }}>
        Backup
      </h2>

      <div className="bk-warn" role="note">
        <b>Backups contain secret.key.</b> It unlocks the SSH key and printer passwords stored in Foreman, so anyone with a backup
        file can read them. Files are written readable by you only; keep them somewhere private, or turn on encryption below.
      </div>

      <p className="auth__hint" style={{ margin: "0 0 var(--fw-space-3)" }}>
        Each backup holds the Foreman database{ov.spoolman_included ? ", the Spoolman database" : ""}, secret.key, settings and
        theme files (fonts, custom.css). Data folder: <span className="auth__mono">{ov.data_dir}</span>
      </p>

      <div className="bk-form">
        <h3 className="auth__cap" style={{ ...cap, marginTop: 0 }}>
          Folder
        </h3>
        <div className="bk-field">
          <input
            className="auth__input auth__mono"
            aria-label="Backup folder"
            value={draft.dir}
            placeholder={ov.default_dir}
            onChange={(e) => {
              set({ dir: e.target.value });
              setDirCheck(null);
            }}
          />
          <Button variant="secondary" disabled={busy || !draft.dir.trim()} onClick={() => void checkDir()}>
            Check folder
          </Button>
        </div>
        <span className={`auth__hint ${dirCheck ? (dirCheck.ok ? "auth__hint--ok" : "auth__hint--warn") : ""}`} role="status">
          {dirCheck ? `${dirCheck.ok ? "✓" : "⚠"} ${dirCheck.message}` : "A folder on this machine. A network share works if you mount it yourself."}
        </span>

        <h3 className="auth__cap" style={cap}>
          Schedule
        </h3>
        <div className="bk-field">
          <select className="bk-select" aria-label="Schedule" value={draft.schedule} onChange={(e) => set({ schedule: e.target.value as BackupSettingsDto["schedule"] })}>
            <option value="off">Off</option>
            <option value="daily">Daily</option>
            <option value="weekly">Weekly</option>
          </select>
          {draft.schedule === "weekly" && (
            <select className="bk-select" aria-label="Weekday" value={draft.weekday} onChange={(e) => set({ weekday: Number(e.target.value) })}>
              {WEEKDAYS.map((d, i) => (
                <option key={d} value={i + 1}>
                  {d}
                </option>
              ))}
            </select>
          )}
          {draft.schedule !== "off" && (
            <input type="time" className="bk-select" aria-label="Time" value={draft.time} onChange={(e) => set({ time: e.target.value })} />
          )}
          <label className="auth__hint bk-field">
            Keep
            <input
              type="number"
              className="bk-small"
              min={1}
              max={365}
              aria-label="Backups to keep"
              value={draft.retention}
              onChange={(e) => set({ retention: Math.max(1, Number(e.target.value) || 1) })}
            />
            newest backups (older ones in this folder are deleted)
          </label>
        </div>

        <label className="auth__check">
          <input type="checkbox" checked={draft.include_library} onChange={(e) => set({ include_library: e.target.checked })} />
          <span>
            Include Library files{" "}
            <span className="auth__hint">
              (adds {ov.library_files} files, {fmtBytes(ov.library_bytes)} to every backup)
            </span>
          </span>
        </label>

        <label className="auth__check">
          <input type="checkbox" checked={draft.encrypt} onChange={(e) => set({ encrypt: e.target.checked })} />
          <span>Encrypt with a passphrase</span>
        </label>
        {draft.encrypt && (
          <div className="bk-field">
            <input
              type="password"
              className="auth__input"
              autoComplete="new-password"
              aria-label="Backup passphrase"
              placeholder={ov.has_passphrase ? "Saved — type to replace" : "Passphrase (8+ characters)"}
              value={pass}
              onChange={(e) => setPass(e.target.value)}
            />
            <span className="auth__hint" style={{ flexBasis: "100%" }}>
              Files end in .zip.age (the open age format). Without the passphrase a backup cannot be opened, by Foreman or anyone else.
              By hand: <span className="auth__mono">age -d -o backup.zip file.zip.age</span>
            </span>
          </div>
        )}

        <div className="bk-actions">
          <Button variant="primary" disabled={busy || !dirty} onClick={() => void save()}>
            Save
          </Button>
          <Button variant="secondary" disabled={busy || dirty} title={dirty ? "Save first" : undefined} onClick={() => void runNow()}>
            {busy ? "Working…" : "Run now"}
          </Button>
        </div>
        {error && (
          <p className="auth__hint auth__hint--warn" role="alert" style={{ margin: 0 }}>
            {error}
          </p>
        )}
        {note && (
          <p className="auth__hint auth__hint--ok" role="status" style={{ margin: 0 }}>
            {note}
          </p>
        )}
        <p className="auth__hint" style={{ margin: 0 }}>
          Schedule: {scheduleText(ov.settings)}
          {ov.next_run ? ` · next ${fmtWhen(ov.next_run)}` : ""}
          {last &&
            (last.ok
              ? ` · last backup ${fmtWhen(last.at)} ✓`
              : ` · last attempt ${fmtWhen(last.at)} failed: ${last.error ?? ""}`)}
        </p>
      </div>

      <h3 className="auth__cap" style={cap}>
        Backups in this folder
      </h3>
      {ov.backups.length === 0 ? (
        <p className="auth__hint">None yet.</p>
      ) : (
        <BackupList entries={ov.backups} onRestore={setTarget} />
      )}

      {ov.snapshots.length > 0 && (
        <>
          <h3 className="auth__cap" style={cap}>
            Saved before a restore
          </h3>
          <p className="auth__hint" style={{ margin: "0 0 var(--fw-space-2)" }}>
            Restoring one of these undoes that restore.
          </p>
          <BackupList entries={ov.snapshots} onRestore={setTarget} />
        </>
      )}

      <h3 className="auth__cap" style={cap}>
        Restore a backup from elsewhere
      </h3>
      <div className="bk-field">
        <input
          className="auth__input auth__mono"
          aria-label="Backup file path"
          placeholder="/path/to/foreman-backup-….zip"
          value={otherPath}
          onChange={(e) => setOtherPath(e.target.value)}
        />
        <Button variant="secondary" disabled={!otherPath.trim()} onClick={() => setTarget(otherPath.trim())}>
          Restore…
        </Button>
      </div>

      {target && <RestorePanel key={target} path={target} onClose={() => setTarget(null)} />}
    </section>
  );
}

function BackupList({ entries, onRestore }: { entries: BackupEntry[]; onRestore: (path: string) => void }) {
  return (
    <div className="security__list">
      {entries.map((b) => (
        <div key={b.path} className="security__row">
          <div className="bk-entry">
            <b className="auth__mono">{b.file}</b>
            <div className="auth__hint">
              {b.manifest ? fmtWhen(b.manifest.created_at) : "no manifest"} · {fmtBytes(b.size)}
              {b.encrypted ? " · encrypted" : ""}
              {b.manifest?.library_included ? " · with Library" : ""}
              {b.manifest ? ` · ${countsLine(b.manifest.counts)}` : ""}
            </div>
          </div>
          <Button variant="secondary" onClick={() => onRestore(b.path)}>
            Restore…
          </Button>
        </div>
      ))}
    </div>
  );
}

function RestorePanel({ path, onClose }: { path: string; onClose: () => void }) {
  const encrypted = path.endsWith(".age");
  const [pass, setPass] = useState("");
  const [info, setInfo] = useState<InspectResult | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [phase, setPhase] = useState<"check" | "restarting" | "back">("check");

  const check = useCallback(async () => {
    setBusy(true);
    setError("");
    try {
      setInfo(await backupApi.inspect(path, encrypted ? pass : undefined));
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  }, [path, pass, encrypted]);

  useEffect(() => {
    if (!encrypted) void check();
    // Only on open; encrypted backups wait for the passphrase.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function restore() {
    setBusy(true);
    setError("");
    try {
      await backupApi.restore(path, encrypted ? pass : undefined);
      setPhase("restarting");
      // Wait for the server to go away and come back, then reload the app.
      let down = false;
      for (let i = 0; i < 120; i++) {
        await new Promise((r) => setTimeout(r, 1000));
        const up = await backupApi.alive();
        if (!up) down = true;
        if (up && (down || i >= 10)) break;
      }
      setPhase("back");
      window.location.reload();
    } catch (e) {
      setError(errText(e));
      setBusy(false);
    }
  }

  return (
    <div className="bk-restore" role="region" aria-label="Restore backup" style={{ marginTop: "var(--fw-space-4)" }}>
      <b>Restore {path.split(/[\\/]/).pop()}</b>
      {encrypted && !info && (
        <form
          className="bk-field"
          onSubmit={(e) => {
            e.preventDefault();
            void check();
          }}
        >
          <input
            type="password"
            className="auth__input"
            autoComplete="off"
            aria-label="Passphrase of this backup"
            placeholder="Passphrase of this backup"
            value={pass}
            onChange={(e) => setPass(e.target.value)}
          />
          <Button type="submit" variant="secondary" disabled={busy || !pass}>
            Check backup
          </Button>
        </form>
      )}
      {busy && !info && phase === "check" && <span className="auth__hint">Checking…</span>}
      {info && (
        <>
          <table className="bk-table">
            <thead>
              <tr>
                <th scope="col"></th>
                <th scope="col">In the backup</th>
                <th scope="col">Now</th>
              </tr>
            </thead>
            <tbody>
              {compareRows(info).map((r) => (
                <tr key={r.label}>
                  <th scope="row">{r.label}</th>
                  <td className={r.differs ? "bk-diff" : undefined}>{r.backup}</td>
                  <td>{r.now}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {info.problem ? (
            <div className="bk-danger" role="alert">
              {info.problem}
            </div>
          ) : (
            <p className="auth__hint" style={{ margin: 0 }}>
              Foreman first saves the current state (listed under “Saved before a restore”, so this can be undone), then restarts with
              this backup. Printers keep printing; Foreman reconnects to them. You may have to sign in again.
            </p>
          )}
        </>
      )}
      {error && (
        <p className="auth__hint auth__hint--warn" role="alert" style={{ margin: 0 }}>
          {error}
        </p>
      )}
      {phase === "restarting" && (
        <p className="auth__hint" role="status" style={{ margin: 0 }}>
          Restarting Foreman with the backup… this page reloads when it is back.
        </p>
      )}
      <div className="bk-actions">
        <Button variant="danger" disabled={busy || !info || !!info.problem || phase !== "check"} onClick={() => void restore()}>
          Restore and restart
        </Button>
        <Button variant="secondary" disabled={phase !== "check"} onClick={onClose}>
          Cancel
        </Button>
      </div>
    </div>
  );
}
