import { useCallback, useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { api, ApiError } from "../api/client";
import type { FilesInfo, PrinterSsh, SftpEntry, ShellMacro, TextFile } from "../api/client";
import { ToastStack, useToasts } from "../ui";
import "./PrinterFiles.css";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const TEXT_RE = /\.(cfg|conf|log|sh|md|txt|json|ini|toml|ya?ml|py|service)$|^\.[^.]/i;
const DANGER_RE = /\b(sudo|rm|reboot|shutdown|dd|mkfs)\b/;

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

function errCode(e: unknown): string | null {
  return e instanceof ApiError ? e.code : null;
}

function fmtSize(n: number | null): string {
  if (n == null) return "";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(n < 10240 ? 1 : 0)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

function fmtDate(secs: number | null): string {
  if (secs == null) return "";
  const d = new Date(secs * 1000);
  return d.toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}

function parentOf(p: string): string {
  const i = p.lastIndexOf("/");
  return i <= 0 ? "/" : p.slice(0, i);
}

function joinPath(dir: string, name: string): string {
  return dir === "/" ? `/${name}` : `${dir}/${name}`;
}

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

// ---------------------------------------------------------------------------
// Files tab
// ---------------------------------------------------------------------------

export function FilesTab(props: { printerId: number; firmware: string; isPrinting: boolean }) {
  return <SshTab {...props} mode="files" />;
}

export function TerminalTab(props: { printerId: number; firmware: string }) {
  return <SshTab {...props} isPrinting={false} mode="terminal" />;
}

function SshTab({
  printerId,
  firmware,
  isPrinting,
  mode,
}: {
  printerId: number;
  firmware: string;
  isPrinting: boolean;
  mode: "files" | "terminal";
}) {
  const label = mode === "files" ? "Files" : "Terminal";
  const [info, setInfo] = useState<FilesInfo | null>(null);
  const [connErr, setConnErr] = useState<{ msg: string; code: string | null } | null>(null);
  const { toasts, addToast, removeToast } = useToasts();

  const connect = useCallback(() => {
    setConnErr(null);
    api
      .filesInfo(printerId)
      .then(setInfo)
      .catch((e) => setConnErr({ msg: errText(e), code: errCode(e) }));
  }, [printerId]);

  useEffect(() => {
    if (firmware !== "bambu") connect();
  }, [connect, firmware]);

  if (firmware === "bambu") {
    return (
      <div className="pf" role="tabpanel" aria-label={label}>
        <div className="pf-note">
          {mode === "files"
            ? "Bambu printers have no SSH. Files are browsed over the printer's built-in FTPS (SD card, LAN mode)."
            : "Bambu printers have no SSH, so there is no terminal."}
        </div>
      </div>
    );
  }

  const trust = async () => {
    try {
      await api.trustHostKey(printerId);
      addToast("Trusted the new host key");
      connect();
    } catch (e) {
      addToast(`Trust failed: ${errText(e)}`);
    }
  };

  const conn = info ? `${info.user}@${info.host}:${info.port}` : "connecting…";

  return (
    <div className="pf" role="tabpanel" aria-label={label}>
      <div className="pf-topbar">
        <span className="pf-conn">{conn}</span>
        {info && <span className="pf-proto">● {mode === "files" ? "SFTP over SSH" : "SSH"}</span>}
      </div>

      {connErr && (
        <div className={`pf-note ${connErr.code === "host_key_changed" ? "pf-note--bad" : "pf-note--warn"}`}>
          <span style={{ flex: "1 1 260px" }}>
            {connErr.code === "ssh_not_configured" ? "SSH is not set up for this printer. Add it in the Config tab → SSH." : connErr.msg}
          </span>
          {connErr.code === "host_key_changed" && (
            <button type="button" className="pf-btn pf-btn--danger" onClick={trust}>
              Trust new host key
            </button>
          )}
          {connErr.code !== "ssh_not_configured" && (
            <button type="button" className="pf-btn" onClick={connect}>
              Retry
            </button>
          )}
        </div>
      )}

      {info && mode === "files" && (
        <FileBrowser printerId={printerId} info={info} isPrinting={isPrinting} toast={addToast} />
      )}
      {info && mode === "terminal" && <SshTerminal printerId={printerId} info={info} />}
      {info && mode === "terminal" && (
        <span className="pf-hint">
          A real terminal (xterm.js, SSH from the Foreman server). Runs as {info.user}; commands you type and macros you
          run are recorded in the printer's audit log.
        </span>
      )}

      <ToastStack toasts={toasts} onDismiss={removeToast} />
    </div>
  );
}

// ---------------------------------------------------------------------------
// File browser + editor
// ---------------------------------------------------------------------------

function FileBrowser({
  printerId,
  info,
  isPrinting,
  toast,
}: {
  printerId: number;
  info: FilesInfo;
  isPrinting: boolean;
  toast: (text: string) => void;
}) {
  const roots = info.roots;
  const [path, setPath] = useState(roots.config);
  const [entries, setEntries] = useState<SftpEntry[]>([]);
  const [listErr, setListErr] = useState("");
  const [loading, setLoading] = useState(false);
  const [sel, setSel] = useState<SftpEntry | null>(null);
  const [file, setFile] = useState<TextFile | null>(null);
  const [fileErr, setFileErr] = useState("");
  const [edit, setEdit] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [delArm, setDelArm] = useState(false);
  const [restartArm, setRestartArm] = useState(false);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [newFolder, setNewFolder] = useState<string | null>(null);
  const uploadRef = useRef<HTMLInputElement>(null);

  const load = useCallback(
    (p: string) => {
      setLoading(true);
      setListErr("");
      api
        .listFiles(printerId, p)
        .then(setEntries)
        .catch((e) => {
          setEntries([]);
          setListErr(errText(e));
        })
        .finally(() => setLoading(false));
    },
    [printerId],
  );

  useEffect(() => load(path), [load, path]);

  const go = (p: string) => {
    setPath(p);
    setSel(null);
    setFile(null);
    setEdit(null);
    setDelArm(false);
    setRestartArm(false);
    setRenaming(null);
  };

  const open = (e: SftpEntry) => {
    if (e.kind === "directory") {
      go(e.path);
      return;
    }
    setSel(e);
    setFile(null);
    setFileErr("");
    setEdit(null);
    setDelArm(false);
    setRestartArm(false);
    setRenaming(null);
    if (TEXT_RE.test(e.name)) {
      api
        .readFile(printerId, e.path)
        .then(setFile)
        .catch((err) => setFileErr(errText(err)));
    }
  };

  // Breadcrumbs relative to home when inside it.
  const home = roots.home;
  const inHome = path === home || path.startsWith(`${home}/`);
  const rel = inHome ? path.slice(home.length) : path;
  const parts = rel.split("/").filter(Boolean);
  const base = inHome ? home : "";
  const crumbs = [{ label: inHome ? "~" : "/", path: inHome ? home : "/" }].concat(
    parts.map((n, i) => ({ label: n, path: `${base}/${parts.slice(0, i + 1).join("/")}` })),
  );

  const places = [
    { label: "config", path: roots.config },
    { label: "gcodes", path: roots.gcodes },
    { label: "logs", path: roots.logs },
  ];

  const isText = !!sel && TEXT_RE.test(sel.name);
  const original = file?.content ?? "";
  const text = edit ?? original;
  const dirty = edit !== null && edit !== original;
  const readOnly = !file || file.read_only;
  const isKlipperCfg = !!sel && sel.path.startsWith(`${roots.config}/`) && sel.name.endsWith(".cfg");
  const isLog = !!sel && (sel.path === roots.logs || sel.path.startsWith(`${roots.logs}/`));

  const save = async (restart: boolean) => {
    if (!sel) return;
    setBusy(true);
    try {
      if (restart) {
        await api.saveAndRestart(printerId, sel.path, text);
        toast(`Saved ${sel.name} · .bak kept · FIRMWARE_RESTART sent`);
      } else {
        const r = await api.writeFile(printerId, sel.path, text);
        toast(`Saved ${sel.name}${r.backup ? " · backup .bak kept" : ""}`);
      }
      setFile({ ...(file as TextFile), content: text });
      setEdit(null);
      load(path);
    } catch (e) {
      toast(`${restart ? "Not restarted" : "Save failed"}: ${errText(e)}`);
    } finally {
      setBusy(false);
      setRestartArm(false);
    }
  };

  const del = async () => {
    if (!sel) return;
    if (!delArm) {
      setDelArm(true);
      return;
    }
    setBusy(true);
    try {
      await api.deleteFile(printerId, sel.path);
      toast(`Deleted ${sel.name}`);
      setSel(null);
      setFile(null);
      load(path);
    } catch (e) {
      toast(`Delete failed: ${errText(e)}`);
    } finally {
      setBusy(false);
      setDelArm(false);
    }
  };

  const doRename = async () => {
    if (!sel || renaming == null) return;
    const name = renaming.trim();
    if (!name || name.includes("/") || name === sel.name) {
      setRenaming(null);
      return;
    }
    setBusy(true);
    try {
      const r = await api.renameFile(printerId, sel.path, joinPath(parentOf(sel.path), name));
      toast(`Renamed to ${name}`);
      setSel({ ...sel, name, path: r.path });
      setRenaming(null);
      load(path);
    } catch (e) {
      toast(`Rename failed: ${errText(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const doMkdir = async () => {
    const name = (newFolder ?? "").trim();
    setNewFolder(null);
    if (!name || name.includes("/")) return;
    try {
      await api.mkdir(printerId, joinPath(path, name));
      toast(`Created ${name}`);
      load(path);
    } catch (e) {
      toast(`New folder failed: ${errText(e)}`);
    }
  };

  const doUpload = async (files: FileList | null) => {
    if (!files) return;
    for (const f of Array.from(files)) {
      try {
        await api.uploadFile(printerId, joinPath(path, f.name), f);
        toast(`Uploaded ${f.name} to ${path}`);
      } catch (e) {
        toast(`Upload of ${f.name} failed: ${errText(e)}`);
      }
    }
    if (uploadRef.current) uploadRef.current.value = "";
    load(path);
  };

  const inLogs = path === roots.logs || path.startsWith(`${roots.logs}/`);

  const stateLabel = isLog
    ? "read-only · log"
    : file?.truncated
      ? "read-only · large file (tail)"
      : dirty
        ? "unsaved changes"
        : "saved";

  const edNote = isLog
    ? `Logs are read-only. Download for the full file.${file?.truncated ? " Showing the last 2 MB." : ""}`
    : file?.truncated
      ? "Too large to edit here — showing the last 2 MB. Download it instead."
      : isKlipperCfg
        ? "Saving keeps a .bak copy. Klipper only reads it after FIRMWARE_RESTART."
        : "Saving keeps a .bak copy next to the file.";

  const restartLabel = isPrinting
    ? "Restart refused while printing"
    : restartArm
      ? "Click again to save + restart"
      : "Save + FIRMWARE_RESTART";

  return (
    <div className="pf-split">
      <section aria-label="File browser" className="pf-panel pf-panel--browser">
        <div className="pf-bar">
          {crumbs.map((c) => (
            <button key={c.path} type="button" className="pf-crumb" onClick={() => go(c.path)}>
              {c.label}
            </button>
          ))}
          <span style={{ flex: 1 }} />
          {places.map((q) => (
            <button key={q.label} type="button" className="pf-btn pf-btn--sm" onClick={() => go(q.path)}>
              {q.label}
            </button>
          ))}
        </div>
        <div className="pf-bar">
          <button type="button" className="pf-btn" onClick={() => go(parentOf(path))} disabled={path === "/"}>
            ↑ Up
          </button>
          <button type="button" className="pf-btn" onClick={() => uploadRef.current?.click()} disabled={inLogs}>
            Upload…
          </button>
          <input
            ref={uploadRef}
            type="file"
            multiple
            hidden
            onChange={(e) => void doUpload(e.target.files)}
          />
          {newFolder === null ? (
            <button type="button" className="pf-btn" onClick={() => setNewFolder("")} disabled={inLogs}>
              New folder
            </button>
          ) : (
            <input
              className="pf-input"
              autoFocus
              aria-label="New folder name"
              placeholder="folder name ↵"
              value={newFolder}
              onChange={(e) => setNewFolder(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void doMkdir();
                if (e.key === "Escape") setNewFolder(null);
              }}
              onBlur={() => setNewFolder(null)}
            />
          )}
          <button type="button" className="pf-btn" onClick={() => load(path)}>
            Refresh
          </button>
        </div>
        <div role="list" className="pf-list">
          {loading && <div className="pf-empty">Loading…</div>}
          {!loading && listErr && <div className="pf-empty pf-err">{listErr}</div>}
          {!loading && !listErr && entries.length === 0 && <div className="pf-empty">Empty folder.</div>}
          {!loading &&
            entries.map((e) => (
              <button
                key={e.path}
                type="button"
                role="listitem"
                className="pf-row"
                aria-current={sel?.path === e.path}
                onClick={() => open(e)}
              >
                <span className="pf-row__icon" aria-hidden="true">
                  {e.kind === "directory" ? "▸" : "·"}
                </span>
                <span className="pf-row__name">
                  {e.name}
                  {e.kind === "directory" ? "/" : ""}
                </span>
                <span className="pf-row__meta pf-row__meta--size">{fmtSize(e.size)}</span>
                <span className="pf-row__meta">{fmtDate(e.modified)}</span>
              </button>
            ))}
        </div>
      </section>

      <section aria-label="File" className={`pf-panel pf-panel--file${dirty ? " pf-panel--dirty" : ""}`}>
        {!sel ? (
          <div className="pf-empty">
            Pick a file. Text files (.cfg, .conf, .log, .sh) open here to read or edit; others can be downloaded.
          </div>
        ) : (
          <>
            <div className="pf-bar">
              {renaming === null ? (
                <b className="pf-fname">{sel.path}</b>
              ) : (
                <input
                  className="pf-input"
                  style={{ flex: "1 1 140px" }}
                  autoFocus
                  aria-label="New name"
                  value={renaming}
                  onChange={(e) => setRenaming(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") void doRename();
                    if (e.key === "Escape") setRenaming(null);
                  }}
                />
              )}
              {isText && file && (
                <span className={`pf-state${dirty ? " pf-state--dirty" : ""}`}>{stateLabel}</span>
              )}
              <a className="pf-btn" href={api.downloadUrl(printerId, sel.path)} download={sel.name}>
                Download
              </a>
              {renaming === null ? (
                <button type="button" className="pf-btn" onClick={() => setRenaming(sel.name)} disabled={isLog || busy}>
                  Rename
                </button>
              ) : (
                <button type="button" className="pf-btn" onClick={() => void doRename()} disabled={busy}>
                  OK
                </button>
              )}
              <button type="button" className="pf-btn pf-btn--danger" onClick={() => void del()} disabled={isLog || busy}>
                {delArm ? "Delete? Click again" : "Delete"}
              </button>
            </div>
            {isText ? (
              <>
                {fileErr ? (
                  <div className="pf-empty pf-err">{fileErr}</div>
                ) : !file ? (
                  <div className="pf-empty">Loading…</div>
                ) : (
                  <textarea
                    className="pf-editor"
                    aria-label="File contents"
                    spellCheck={false}
                    readOnly={readOnly}
                    value={text}
                    onChange={(e) => setEdit(e.target.value)}
                  />
                )}
                <div className="pf-bar pf-bar--bottom">
                  <span className="pf-ednote">{edNote}</span>
                  <button type="button" className="pf-btn" onClick={() => setEdit(null)} disabled={!dirty || busy}>
                    Revert
                  </button>
                  <button
                    type="button"
                    className="pf-btn--primary"
                    onClick={() => void save(false)}
                    disabled={!dirty || busy || readOnly}
                  >
                    Save
                  </button>
                  {isKlipperCfg && (
                    <button
                      type="button"
                      className="pf-btn"
                      title={isPrinting ? "FIRMWARE_RESTART is refused while the printer is printing" : undefined}
                      disabled={isPrinting || busy || readOnly}
                      onClick={() => (restartArm ? void save(true) : setRestartArm(true))}
                    >
                      {restartLabel}
                    </button>
                  )}
                </div>
              </>
            ) : (
              <div className="pf-empty">
                {/\.(gcode|3mf)$/i.test(sel.name)
                  ? "G-code/3MF. Download it, or open it in Library to print or queue it."
                  : "Binary file. Download to open it."}
              </div>
            )}
          </>
        )}
      </section>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Terminal (xterm.js ⇄ /ws/printers/{id}/terminal ⇄ SSH PTY)
// ---------------------------------------------------------------------------

const BUILTIN_SNIPS = [
  { label: "uptime", cmd: "uptime" },
  { label: "df -h /", cmd: "df -h /" },
  { label: "klipper status", cmd: "systemctl status klipper --no-pager" },
  { label: "tail klippy.log", cmd: "tail -n 20 ~/printer_data/logs/klippy.log" },
];

interface SnipForm {
  id: number | null;
  name: string;
  commands: string;
  all: boolean;
  ask: boolean;
}

function xtermTheme() {
  return {
    background: cssVar("--fw-surface"),
    foreground: cssVar("--fw-text"),
    cursor: cssVar("--fw-accent"),
    cursorAccent: cssVar("--fw-surface"),
    selectionBackground: cssVar("--fw-info-bg"),
  };
}

function SshTerminal({ printerId, info }: { printerId: number; info: FilesInfo }) {
  const hostRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const [status, setStatus] = useState<"connecting" | "ready" | "closed" | "error">("connecting");
  const [statusMsg, setStatusMsg] = useState("");
  const [size, setSize] = useState({ cols: 120, rows: 32 });
  const [paste, setPaste] = useState<string[] | null>(null);
  const [macros, setMacros] = useState<ShellMacro[]>([]);
  const [armed, setArmed] = useState<number | null>(null);
  const [form, setForm] = useState<SnipForm | null>(null);
  const [formErr, setFormErr] = useState("");
  const [session, setSession] = useState(0);

  const loadMacros = useCallback(() => {
    api.listShellMacros(printerId).then(setMacros).catch(() => setMacros([]));
  }, [printerId]);

  useEffect(loadMacros, [loadMacros]);

  const send = useCallback((msg: unknown) => {
    const ws = wsRef.current;
    if (ws && ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(msg));
  }, []);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const term = new Terminal({
      fontFamily: cssVar("--fw-mono") || "monospace",
      fontSize: 12,
      cursorBlink: true,
      theme: xtermTheme(),
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    fit.fit();
    termRef.current = term;
    setSize({ cols: term.cols, rows: term.rows });

    const ws = new WebSocket(api.terminalWsUrl(printerId, term.cols, term.rows));
    ws.binaryType = "arraybuffer";
    wsRef.current = ws;
    setStatus("connecting");
    setStatusMsg("");
    ws.onmessage = (ev) => {
      if (typeof ev.data === "string") {
        try {
          const m = JSON.parse(ev.data) as { type: string; message?: string };
          if (m.type === "ready") {
            setStatus("ready");
            term.focus();
          } else if (m.type === "error") {
            setStatus("error");
            setStatusMsg(m.message ?? "error");
            term.writeln(`\r\n[foreman] ${m.message ?? "error"}`);
          } else if (m.type === "exit") {
            setStatus("closed");
          }
        } catch {
          /* ignore malformed control frame */
        }
      } else {
        term.write(new Uint8Array(ev.data as ArrayBuffer));
      }
    };
    ws.onclose = () => setStatus((s) => (s === "error" ? s : "closed"));

    const onData = term.onData((d) => {
      if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ type: "input", data: d }));
    });

    // Multi-line paste needs confirmation; intercept before xterm's own handler.
    const onPaste = (e: ClipboardEvent) => {
      const t = e.clipboardData?.getData("text") ?? "";
      if (/\r?\n/.test(t.trim())) {
        e.preventDefault();
        e.stopImmediatePropagation();
        setPaste(t.trim().split(/\r?\n/));
      }
    };
    host.addEventListener("paste", onPaste, true);

    const ro = new ResizeObserver(() => {
      fit.fit();
      setSize({ cols: term.cols, rows: term.rows });
      if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ type: "resize", cols: term.cols, rows: term.rows }));
    });
    ro.observe(host);

    // Follow theme/mode switches on <html>.
    const mo = new MutationObserver(() => {
      term.options.theme = xtermTheme();
    });
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-fw-theme", "data-fw-mode"] });

    return () => {
      ro.disconnect();
      mo.disconnect();
      host.removeEventListener("paste", onPaste, true);
      onData.dispose();
      ws.close();
      term.dispose();
      wsRef.current = null;
      termRef.current = null;
    };
  }, [printerId, session]);

  const typeLine = (cmd: string) => {
    send({ type: "input", data: `${cmd}\r` });
    termRef.current?.focus();
  };

  const runMacro = (m: ShellMacro) => {
    if (m.ask_before_run && armed !== m.id) {
      setArmed(m.id);
      return;
    }
    setArmed(null);
    send({ type: "macro", id: m.id });
    termRef.current?.focus();
  };

  const saveForm = async () => {
    if (!form) return;
    setFormErr("");
    const body = {
      name: form.name.trim(),
      commands: form.commands,
      scope: form.all ? "all_ssh" : "printer",
      ask_before_run: form.ask,
    };
    try {
      if (form.id == null) await api.createShellMacro(printerId, body);
      else await api.updateShellMacro(printerId, form.id, body);
      setForm(null);
      loadMacros();
    } catch (e) {
      setFormErr(errText(e));
    }
  };

  const deleteForm = async () => {
    if (!form || form.id == null) return;
    try {
      await api.deleteShellMacro(printerId, form.id);
      setForm(null);
      loadMacros();
    } catch (e) {
      setFormErr(errText(e));
    }
  };

  const statusText =
    status === "ready" ? "" : status === "connecting" ? " · connecting…" : status === "error" ? ` · ${statusMsg}` : " · disconnected";

  return (
    <section aria-label="SSH terminal" className="pf-term">
      <div className="pf-term__bar">
        <span className="pf-term__label">
          {info.user}@{info.host}:{info.port} · xterm {size.cols}×{size.rows}
          {statusText}
        </span>
        {(status === "closed" || status === "error") && (
          <span className="pf-snip">
            <button type="button" onClick={() => setSession((n) => n + 1)}>
              Reconnect
            </button>
          </span>
        )}
        {BUILTIN_SNIPS.map((s) => (
          <span key={s.label} className="pf-snip">
            <button type="button" title={s.cmd} onClick={() => typeLine(s.cmd)} disabled={status !== "ready"}>
              {s.label}
            </button>
          </span>
        ))}
        {macros.map((m) => (
          <span key={m.id} className={`pf-snip pf-snip--user${armed === m.id ? " pf-snip--armed" : ""}`}>
            <button type="button" title={m.commands} onClick={() => runMacro(m)} disabled={status !== "ready"}>
              {armed === m.id ? `run “${m.name}”? click again` : m.name}
            </button>
            <button
              type="button"
              aria-label={`Edit ${m.name}`}
              onClick={() =>
                setForm({ id: m.id, name: m.name, commands: m.commands, all: m.scope === "all_ssh", ask: m.ask_before_run })
              }
            >
              ✎
            </button>
          </span>
        ))}
        <span className="pf-snip">
          <button
            type="button"
            className="pf-btn--dashed"
            onClick={() => setForm({ id: null, name: "", commands: "", all: false, ask: false })}
          >
            + Add
          </button>
        </span>
      </div>

      {form && (
        <div className="pf-snipform">
          <label style={{ flex: "0 1 160px" }}>
            Button name
            <input
              className="pf-input"
              value={form.name}
              placeholder="e.g. restart cam"
              onChange={(e) => setForm({ ...form, name: e.target.value })}
            />
          </label>
          <label style={{ flex: "1 1 260px" }}>
            Command
            <textarea
              rows={3}
              spellCheck={false}
              value={form.commands}
              placeholder={"sudo systemctl restart crowsnest\none command per line, or a whole script"}
              onChange={(e) => {
                const v = e.target.value;
                setForm({ ...form, commands: v, ask: form.ask || DANGER_RE.test(v) });
              }}
            />
          </label>
          <label>
            Show on
            <select
              className="pf-input"
              value={form.all ? "all" : "this"}
              onChange={(e) => setForm({ ...form, all: e.target.value === "all" })}
            >
              <option value="this">This printer</option>
              <option value="all">All SSH printers</option>
            </select>
          </label>
          <label className="pf-snipform__check">
            <input
              type="checkbox"
              checked={form.ask}
              disabled={DANGER_RE.test(form.commands)}
              onChange={() => setForm({ ...form, ask: !form.ask })}
            />
            Ask before running
          </label>
          <button
            type="button"
            className="pf-btn--primary"
            onClick={() => void saveForm()}
            disabled={!form.name.trim() || !form.commands.trim()}
          >
            {form.id == null ? "Add button" : "Save"}
          </button>
          {form.id != null && (
            <button type="button" className="pf-btn pf-btn--sm pf-btn--danger" onClick={() => void deleteForm()}>
              Delete
            </button>
          )}
          <button type="button" className="pf-btn pf-btn--sm" onClick={() => setForm(null)}>
            Cancel
          </button>
          {formErr && <span className="pf-err">{formErr}</span>}
          <span className="pf-snipform__note">
            Saved in Foreman, not on the printer. “All SSH printers” shows it on every printer you can SSH into. Runs in
            this terminal so you see the output; the audit log records it.
          </span>
        </div>
      )}

      <div ref={hostRef} className="pf-xterm" />

      {paste && (
        <div className="pf-paste" role="alertdialog" aria-label="Confirm multi-line paste">
          <span className="pf-paste__title">
            Paste {paste.length} lines? They run one after another, like typing each and pressing Enter.
          </span>
          <pre className="pf-paste__lines">{paste.join("\n")}</pre>
          <div style={{ display: "flex", gap: 6 }}>
            <button
              type="button"
              className="pf-btn--primary"
              onClick={() => {
                send({ type: "input", data: `${paste.join("\r")}\r` });
                setPaste(null);
                termRef.current?.focus();
              }}
            >
              Run all {paste.length}
            </button>
            <button
              type="button"
              className="pf-btn pf-btn--sm"
              onClick={() => {
                send({ type: "input", data: paste.join(" && ") });
                setPaste(null);
                termRef.current?.focus();
              }}
            >
              Paste as one line
            </button>
            <button
              type="button"
              className="pf-btn pf-btn--sm"
              onClick={() => {
                setPaste(null);
                termRef.current?.focus();
              }}
            >
              Cancel
            </button>
          </div>
        </div>
      )}
    </section>
  );
}

// ---------------------------------------------------------------------------
// SSH settings (Config tab)
// ---------------------------------------------------------------------------

export function SshSettings({ printerId, firmware }: { printerId: number; firmware: string }) {
  const [cfg, setCfg] = useState<PrinterSsh | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [user, setUser] = useState("pi");
  const [port, setPort] = useState(22);
  const [auth, setAuth] = useState<"key" | "password">("key");
  const [password, setPassword] = useState("");
  const [pubKey, setPubKey] = useState("");
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [busy, setBusy] = useState(false);

  const apply = (c: PrinterSsh) => {
    setCfg(c);
    setUser(c.ssh_user);
    setPort(c.ssh_port);
    setAuth(c.auth_type === "password" ? "password" : "key");
  };

  useEffect(() => {
    if (firmware === "bambu") return;
    api
      .getSsh(printerId)
      .then(apply)
      .catch(() => setCfg(null))
      .finally(() => setLoaded(true));
    api
      .getSshPublicKey()
      .then((r) => setPubKey(r.public_key))
      .catch(() => {});
  }, [printerId, firmware]);

  if (firmware === "bambu") return null;

  const save = async () => {
    setBusy(true);
    setMsg(null);
    try {
      const c = await api.upsertSsh(printerId, {
        ssh_user: user.trim(),
        ssh_port: port,
        auth_type: auth,
        password: auth === "password" && password ? password : null,
      });
      apply(c);
      setPassword("");
      setMsg({ ok: true, text: "SSH settings saved" });
    } catch (e) {
      setMsg({ ok: false, text: errText(e) });
    } finally {
      setBusy(false);
    }
  };

  const test = async () => {
    setBusy(true);
    setMsg(null);
    try {
      const r = await api.testSsh(printerId);
      setMsg({ ok: true, text: `Connected as ${r.user}@${r.host} · home ${r.home}` });
      apply(await api.getSsh(printerId));
    } catch (e) {
      setMsg({ ok: false, text: errText(e) });
      api.getSsh(printerId).then(apply).catch(() => {});
    } finally {
      setBusy(false);
    }
  };

  const trust = async () => {
    setBusy(true);
    try {
      apply(await api.trustHostKey(printerId));
      setMsg({ ok: true, text: "New host key trusted" });
    } catch (e) {
      setMsg({ ok: false, text: errText(e) });
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    if (!confirm("Remove SSH settings for this printer?")) return;
    try {
      await api.deleteSsh(printerId);
      setCfg(null);
      setMsg({ ok: true, text: "SSH settings removed" });
    } catch (e) {
      setMsg({ ok: false, text: errText(e) });
    }
  };

  const copy = () => {
    navigator.clipboard
      .writeText(pubKey)
      .then(() => setMsg({ ok: true, text: "Public key copied" }))
      .catch(() => setMsg({ ok: false, text: "Copy failed — select the key and copy it manually" }));
  };

  return (
    <section className="pf-ssh" aria-label="SSH">
      <h2>SSH</h2>
      <span className="pf-hint">
        Used for the Files tab (SFTP) and the terminal. Connects to the active connection path's host.
      </span>

      {cfg?.pending_host_key_fingerprint && (
        <div className="pf-note pf-note--bad">
          <span style={{ flex: "1 1 260px" }}>
            Host key changed. The printer now presents{" "}
            <span className="pf-mono">{cfg.pending_host_key_fingerprint}</span>; Foreman refuses to connect until you
            trust it. Only do this if you know why it changed (e.g. reinstalled OS).
          </span>
          <button type="button" className="pf-btn pf-btn--danger" onClick={() => void trust()} disabled={busy}>
            Trust new host key
          </button>
        </div>
      )}

      <div className="pf-ssh__grid">
        <label>
          User
          <input className="pf-ssh__field" value={user} onChange={(e) => setUser(e.target.value)} spellCheck={false} />
        </label>
        <label>
          Port
          <input
            className="pf-ssh__field"
            type="number"
            min={1}
            max={65535}
            value={port}
            onChange={(e) => setPort(Number(e.target.value))}
          />
        </label>
      </div>

      <div role="radiogroup" aria-label="Authentication" className="pf-ssh__row">
        <label className="pf-ssh__radio">
          <input type="radio" checked={auth === "key"} onChange={() => setAuth("key")} />
          Foreman key (recommended)
        </label>
        <label className="pf-ssh__radio">
          <input type="radio" checked={auth === "password"} onChange={() => setAuth("password")} />
          Password
        </label>
      </div>

      {auth === "key" ? (
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <span className="pf-hint">
            Add this line to <span className="pf-mono">~/.ssh/authorized_keys</span> on the printer.
          </span>
          <pre className="pf-ssh__key">{pubKey || "…"}</pre>
          <div>
            <button type="button" className="pf-btn pf-btn--sm" onClick={copy} disabled={!pubKey}>
              Copy public key
            </button>
          </div>
        </div>
      ) : (
        <label>
          Password
          <input
            className="pf-ssh__field"
            type="password"
            autoComplete="new-password"
            value={password}
            placeholder={cfg?.has_password ? "•••••• stored (encrypted) — type to replace" : ""}
            onChange={(e) => setPassword(e.target.value)}
          />
        </label>
      )}

      <div className="pf-hint">
        Host key:{" "}
        <span className="pf-mono">
          {cfg?.host_key_fingerprint ?? "not pinned yet — pinned on first connect"}
        </span>
      </div>

      <div className="pf-ssh__row">
        <button type="button" className="pf-btn--primary" onClick={() => void save()} disabled={busy || !user.trim()}>
          {cfg ? "Save SSH" : "Set up SSH"}
        </button>
        <button type="button" className="pf-btn" onClick={() => void test()} disabled={busy || !cfg}>
          Test connection
        </button>
        {cfg && (
          <button type="button" className="pf-btn pf-btn--danger" onClick={() => void remove()} disabled={busy}>
            Remove SSH
          </button>
        )}
        {msg && (
          <span role="status" className={msg.ok ? "pf-ok" : "pf-err"}>
            {msg.text}
          </span>
        )}
        {!loaded && <span className="pf-hint">Loading…</span>}
      </div>
    </section>
  );
}
