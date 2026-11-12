/**
 * "Found on your network" (FleetEmpty.dc.html): LAN scan, pick printers, enter
 * credentials inline, add. Plus manual add by address. Used by the setup wizard's
 * printer step and the empty Fleet page.
 */
import { useCallback, useEffect, useState } from "react";
import { ApiError } from "../api/client";
import { setupApi, type FoundPrinter, type ProbeResult, type ScanResult } from "../api/setup";
import type { FirmwareKind } from "../api/types/FirmwareKind";
import { Button, StatusChip } from "../ui";
import { usePhone } from "../hooks/useMediaQuery";
import {
  addLabel,
  chipFor,
  foundSubline,
  missingFor,
  NO_CREDS,
  readinessChip,
  scannedAgo,
  type Creds,
} from "./setupLogic";

function errText(e: unknown): string {
  return e instanceof ApiError || e instanceof Error ? e.message : String(e);
}

/** What was probed, one line per check. */
export function ProbeChecks({ checks, open }: { checks: FoundPrinter["checks"]; open: boolean }) {
  if (checks.length === 0) return null;
  return (
    <details className="su-checks" open={open}>
      <summary>What was probed</summary>
      <ul>
        {checks.map((c, i) => (
          <li key={i} className={c.ok ? "su-checks__ok" : "su-checks__bad"}>
            <span aria-hidden="true">{c.ok ? "✓" : "✖"}</span> <b>{c.label}</b> {c.detail}
          </li>
        ))}
      </ul>
    </details>
  );
}

export default function FoundPrinters({ onAdded }: { onAdded: () => void }) {
  const phone = usePhone();
  const [scan, setScan] = useState<ScanResult | null>(null);
  const [scanning, setScanning] = useState(false);
  const [scanErr, setScanErr] = useState<string | null>(null);
  const [picked, setPicked] = useState<Record<string, boolean>>({});
  const [creds, setCreds] = useState<Record<string, Creds>>({});
  const [names, setNames] = useState<Record<string, string>>({});
  const [rowErr, setRowErr] = useState<Record<string, string>>({});
  const [adding, setAdding] = useState(false);
  const [manual, setManual] = useState(false);
  const [now, setNow] = useState(() => Date.now());

  const runScan = useCallback(async () => {
    setScanning(true);
    setScanErr(null);
    try {
      const r = await setupApi.discover();
      setScan(r);
      // Pre-pick what can be added without typing anything (as in the reference).
      setPicked((prev) => {
        const next: Record<string, boolean> = {};
        for (const p of r.printers) {
          if (p.added_as != null) continue;
          next[p.key] = prev[p.key] ?? p.needs === "ready";
        }
        return next;
      });
      setNames((prev) => {
        const next = { ...prev };
        for (const p of r.printers) if (!(p.key in next)) next[p.key] = p.name;
        return next;
      });
    } catch (e) {
      setScanErr(errText(e));
    } finally {
      setScanning(false);
      setNow(Date.now());
    }
  }, []);

  useEffect(() => {
    void runScan();
  }, [runScan]);

  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 5000);
    return () => clearInterval(t);
  }, []);

  const printers = scan?.printers ?? [];
  const chosen = printers.filter((p) => picked[p.key] && p.added_as == null);
  const credsOf = (k: string) => creds[k] ?? NO_CREDS;
  const setCred = (k: string, patch: Partial<Creds>) =>
    setCreds((prev) => ({ ...prev, [k]: { ...(prev[k] ?? NO_CREDS), ...patch } }));

  const addChosen = async () => {
    setAdding(true);
    const errs: Record<string, string> = {};
    let any = false;
    for (const p of chosen) {
      const c = credsOf(p.key);
      const missing = missingFor(p, c);
      if (missing) {
        errs[p.key] = missing;
        continue;
      }
      try {
        const r = await setupApi.addPrinter({
          name: (names[p.key] ?? p.name).trim(),
          firmware: p.firmware,
          host: p.url,
          api_key: c.apiKey.trim() || undefined,
          serial: p.serial ?? (c.serial.trim() || undefined),
          access_code: c.accessCode.trim() || undefined,
          force: false,
        });
        any = true;
        setScan((s) =>
          s ? { ...s, printers: s.printers.map((x) => (x.key === p.key ? { ...x, added_as: Number(r.printer.id) } : x)) } : s,
        );
      } catch (e) {
        errs[p.key] = errText(e);
      }
    }
    setRowErr(errs);
    setAdding(false);
    if (any) onAdded();
  };

  return (
    <section className="su-found" aria-label="Printers found on your network">
      <div className="su-found__head">
        <div className="su-found__title">
          <h2>Found on your network</h2>
          <p className="su-muted">
            {scanning
              ? "Scanning…"
              : scan
                ? `${scannedAgo(scan.scanned_at, now)}.${phone ? "" : " Pick the ones to add; Foreman reads bed size, nozzle and tools from each printer."}`
                : scanErr ?? ""}
          </p>
        </div>
        <Button variant="secondary" disabled={scanning} onClick={() => void runScan()}>
          {scanning ? "Scanning…" : "Scan again"}
        </Button>
      </div>

      {scan?.notes.map((n) => (
        <p key={n} className="su-note" role="note">
          {n}
        </p>
      ))}
      {scanErr && (
        <p className="su-error" role="alert">
          {scanErr}
        </p>
      )}

      {printers.map((p) => {
        const chip = readinessChip(p);
        const isPicked = !!picked[p.key] && p.added_as == null;
        const c = credsOf(p.key);
        return (
          <div key={p.key} className={`su-row${isPicked ? " su-row--picked" : ""}`}>
            <label className="su-row__main">
              <input
                type="checkbox"
                className="su-check"
                checked={isPicked}
                disabled={p.added_as != null || adding}
                onChange={(e) => setPicked((prev) => ({ ...prev, [p.key]: e.target.checked }))}
              />
              <span className="su-row__text">
                <b className="su-trunc">{p.name}</b>
                <span className="su-row__sub su-trunc">{foundSubline(p)}</span>
              </span>
              <StatusChip tone={chip.tone} symbol={chip.symbol} word={chip.word} short={chip.short} />
            </label>
            {isPicked && (
              <div className="su-row__fields">
                <label className="su-field">
                  <span>Name</span>
                  <input
                    className="auth__input"
                    value={names[p.key] ?? p.name}
                    onChange={(e) => setNames((prev) => ({ ...prev, [p.key]: e.target.value }))}
                  />
                </label>
                {p.needs === "api_key" && (
                  <label className="su-field">
                    <span>Moonraker API key</span>
                    <input
                      className="auth__input auth__mono"
                      autoComplete="off"
                      value={c.apiKey}
                      onChange={(e) => setCred(p.key, { apiKey: e.target.value })}
                    />
                  </label>
                )}
                {p.firmware === "bambu" && !p.serial && (
                  <label className="su-field">
                    <span>Serial</span>
                    <input
                      className="auth__input auth__mono"
                      value={c.serial}
                      onChange={(e) => setCred(p.key, { serial: e.target.value.toUpperCase() })}
                    />
                  </label>
                )}
                {p.firmware === "bambu" && (
                  <label className="su-field">
                    <span>Access code</span>
                    <input
                      className="auth__input auth__mono"
                      autoComplete="off"
                      inputMode="text"
                      maxLength={8}
                      placeholder="8 characters"
                      value={c.accessCode}
                      onChange={(e) => setCred(p.key, { accessCode: e.target.value.trim() })}
                    />
                  </label>
                )}
              </div>
            )}
            <ProbeChecks checks={p.checks} open={!phone && p.needs !== "ready"} />
            {rowErr[p.key] && (
              <p className="su-error" role="alert">
                {rowErr[p.key]}
              </p>
            )}
          </div>
        );
      })}

      {scan && printers.length === 0 && !scanning && <p className="su-muted">Nothing found yet.</p>}

      <div className="su-actions">
        <Button variant="primary" disabled={adding || chosen.length === 0} onClick={() => void addChosen()}>
          {adding ? "Adding…" : addLabel(chosen.length)}
        </Button>
        <Button variant="secondary" aria-expanded={manual} onClick={() => setManual((v) => !v)}>
          {manual ? "Close manual add" : "Add manually"}
        </Button>
      </div>

      {manual && (
        <ManualAdd
          onAdded={() => {
            onAdded();
            void runScan();
          }}
        />
      )}
    </section>
  );
}

// ---------------------------------------------------------------------------
// Manual add
// ---------------------------------------------------------------------------

function ManualAdd({ onAdded }: { onAdded: () => void }) {
  const [firmware, setFirmware] = useState<FirmwareKind>("moonraker");
  const [host, setHost] = useState("");
  const [name, setName] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [serial, setSerial] = useState("");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [probe, setProbe] = useState<ProbeResult | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [unreachable, setUnreachable] = useState(false);
  const [done, setDone] = useState<string | null>(null);

  const req = () => ({
    firmware,
    host: host.trim(),
    api_key: apiKey.trim() || undefined,
    serial: serial.trim() || undefined,
    access_code: code.trim() || undefined,
  });

  const check = async () => {
    setBusy(true);
    setErr(null);
    setDone(null);
    try {
      const r = await setupApi.probe(req());
      setProbe(r);
      if (!name.trim()) setName(r.printer.name);
      if (r.printer.serial && !serial) setSerial(r.printer.serial);
    } catch (e) {
      setErr(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const add = async (force: boolean) => {
    setBusy(true);
    setErr(null);
    try {
      const r = await setupApi.addPrinter({ ...req(), name: (name || host).trim(), force });
      setDone(`${r.printer.name} added.`);
      setProbe(null);
      setUnreachable(false);
      setHost("");
      setName("");
      setApiKey("");
      setSerial("");
      setCode("");
      onAdded();
    } catch (e) {
      setErr(errText(e));
      setUnreachable(e instanceof ApiError && e.code === "unreachable");
    } finally {
      setBusy(false);
    }
  };

  const chip = probe ? chipFor(probe.printer.needs) : null;
  return (
    <form
      className="su-manual"
      aria-label="Add a printer by address"
      onSubmit={(e) => {
        e.preventDefault();
        void add(false);
      }}
    >
      <div className="su-row__fields">
        <label className="su-field">
          <span>Type</span>
          <select
            className="bk-select"
            value={firmware}
            onChange={(e) => {
              setFirmware(e.target.value as FirmwareKind);
              setProbe(null);
            }}
          >
            <option value="moonraker">Klipper (Moonraker)</option>
            <option value="bambu">Bambu Lab (LAN)</option>
          </select>
        </label>
        <label className="su-field">
          <span>Address</span>
          <input
            className="auth__input auth__mono"
            placeholder={firmware === "bambu" ? "192.168.1.61" : "192.168.1.40 or voron.local:7125"}
            value={host}
            onChange={(e) => setHost(e.target.value)}
            required
          />
        </label>
        <label className="su-field">
          <span>Name</span>
          <input className="auth__input" value={name} onChange={(e) => setName(e.target.value)} placeholder="From the printer" />
        </label>
        {firmware === "moonraker" ? (
          <label className="su-field">
            <span>API key (if Moonraker asks)</span>
            <input className="auth__input auth__mono" autoComplete="off" value={apiKey} onChange={(e) => setApiKey(e.target.value)} />
          </label>
        ) : (
          <>
            <label className="su-field">
              <span>Serial</span>
              <input
                className="auth__input auth__mono"
                value={serial}
                placeholder="Read from the printer"
                onChange={(e) => setSerial(e.target.value.toUpperCase())}
              />
            </label>
            <label className="su-field">
              <span>Access code</span>
              <input
                className="auth__input auth__mono"
                autoComplete="off"
                maxLength={8}
                placeholder="8 characters"
                value={code}
                onChange={(e) => setCode(e.target.value.trim())}
              />
            </label>
          </>
        )}
      </div>
      {probe && chip && (
        <div className="su-probe">
          <StatusChip tone={chip.tone} symbol={chip.symbol} word={chip.word} />
          <ProbeChecks checks={probe.printer.checks} open />
        </div>
      )}
      {err && (
        <p className="su-error" role="alert">
          {err}
        </p>
      )}
      {done && (
        <p className="auth__hint auth__hint--ok" role="status">
          ✓ {done}
        </p>
      )}
      <div className="su-actions">
        <Button variant="secondary" disabled={busy || !host.trim()} onClick={() => void check()}>
          Check
        </Button>
        <Button variant="primary" type="submit" disabled={busy || !host.trim()}>
          {busy ? "Working…" : "Add printer"}
        </Button>
        {unreachable && (
          <Button variant="secondary" disabled={busy} onClick={() => void add(true)}>
            Add anyway (printer off)
          </Button>
        )}
      </div>
    </form>
  );
}
