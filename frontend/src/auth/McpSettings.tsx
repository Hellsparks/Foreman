import { useEffect, useMemo, useState } from "react";
import { api, request } from "../api/client";
import { apiUrl } from "../api/conn";
import type { McpKey } from "../api/types/McpKey";
import type { McpPolicy } from "../api/types/McpPolicy";
import type { McpSettings as McpSettingsDto } from "../api/types/McpSettings";
import type { McpTool } from "../api/types/McpTool";
import type { McpToolClass } from "../api/types/McpToolClass";
import Button from "../ui/Button";
import { clientSnippets, effectiveTools, mcpUrl, preferredBase } from "./mcpLogic";

const CLASS_LABEL: Record<McpToolClass, string> = {
  read: "Read only",
  write: "Changes records",
  destructive: "Destructive — moves printers, starts or cancels prints, deletes data, changes spool counts",
};

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Settings → Security → MCP: endpoint URL, client config, per-key tool access. */
export default function McpSettings({ keysVersion }: { keysVersion: string }) {
  const [data, setData] = useState<McpSettingsDto | null>(null);
  const [addresses, setAddresses] = useState<string[]>([]);
  const [base, setBase] = useState("");
  const [open, setOpen] = useState<number | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    request<McpSettingsDto>("GET", "/api/settings/mcp")
      .then(setData)
      .catch((e) => setError(errText(e)));
  }, [keysVersion]);

  useEffect(() => {
    api
      .serverInfo()
      .then((i) => setAddresses(i.addresses.map((a) => `http://${a.address}`)))
      .catch(() => {});
  }, []);

  const here = useMemo(() => new URL(apiUrl("/"), window.location.href).origin, []);
  const bases = useMemo(() => Array.from(new Set([here, ...addresses])), [here, addresses]);
  const preferred = preferredBase(here, addresses);
  const url = mcpUrl(base || preferred, data?.path ?? "/mcp");
  const snippets = clientSnippets(url);

  const saved = (k: McpKey) =>
    setData((d) => (d ? { ...d, keys: d.keys.map((x) => (x.key.id === k.key.id ? k : x)) } : d));

  return (
    <>
      <h3 className="auth__cap" style={{ margin: "var(--fw-space-6) 0 var(--fw-space-2)", fontWeight: "normal" }}>
        MCP
      </h3>
      <p className="auth__hint" style={{ margin: "0 0 var(--fw-space-3)" }}>
        Lets an AI assistant (Claude, or any MCP client) read your fleet with an API key. Keys reach read-only tools
        until you allow more below.
      </p>

      <div className="security__row">
        <span className="mcp__label">URL</span>
        <code className="auth__mono mcp__url">{url}</code>
        {bases.length > 1 && (
          <select
            className="auth__input mcp__select"
            aria-label="Address for the MCP URL"
            value={base || preferred}
            onChange={(e) => setBase(e.target.value)}
          >
            {bases.map((b) => (
              <option key={b} value={b}>
                {b.replace(/^https?:\/\//, "")}
              </option>
            ))}
          </select>
        )}
        <CopyButton text={url} />
      </div>

      <div className="mcp__snippet">
        <div className="mcp__snippet-head">
          <span className="auth__hint">Claude Code (any shell):</span>
          <CopyButton text={snippets.cli} />
        </div>
        <pre className="auth__mono">{snippets.cli}</pre>
        <div className="mcp__snippet-head">
          <span className="auth__hint">
            Client config (<span className="auth__mono">.mcp.json</span> / <span className="auth__mono">mcpServers</span>):
          </span>
          <CopyButton text={snippets.json} />
        </div>
        <pre className="auth__mono">{snippets.json}</pre>
        <p className="auth__hint" style={{ margin: 0 }}>
          Replace <span className="auth__mono">fmk_…</span> with the key&apos;s secret (shown once when it was created).
        </p>
      </div>

      {data && data.keys.length === 0 && <p className="auth__hint">Create an API key above to connect a client.</p>}
      {data && data.keys.length > 0 && (
        <div className="security__list" style={{ marginTop: "var(--fw-space-3)" }}>
          {data.keys.map((k) => (
            <KeyAccess
              key={k.key.id}
              entry={k}
              tools={data.tools}
              open={open === k.key.id}
              onToggle={() => setOpen((o) => (o === k.key.id ? null : k.key.id))}
              onSaved={saved}
              onError={setError}
            />
          ))}
        </div>
      )}

      {error && (
        <div role="alert" className="auth__alert" style={{ marginTop: "var(--fw-space-3)" }}>
          {error}
        </div>
      )}
    </>
  );
}

function CopyButton({ text }: { text: string }) {
  const [done, setDone] = useState(false);
  return (
    <Button
      variant="small"
      onClick={async () => {
        try {
          await navigator.clipboard.writeText(text);
          setDone(true);
          window.setTimeout(() => setDone(false), 1500);
        } catch {
          /* clipboard blocked — the text is selectable */
        }
      }}
    >
      {done ? "Copied" : "Copy"}
    </Button>
  );
}

function KeyAccess({
  entry,
  tools,
  open,
  onToggle,
  onSaved,
  onError,
}: {
  entry: McpKey;
  tools: McpTool[];
  open: boolean;
  onToggle: () => void;
  onSaved: (k: McpKey) => void;
  onError: (msg: string) => void;
}) {
  const [draft, setDraft] = useState<McpPolicy>(entry.policy);
  const [busy, setBusy] = useState(false);
  useEffect(() => setDraft(entry.policy), [entry.policy]);

  const allowed = new Set(effectiveTools(draft, tools));
  const dirty = JSON.stringify(draft) !== JSON.stringify(entry.policy);
  const destructiveOn = entry.reachable.some((n) => tools.find((t) => t.name === n)?.class === "destructive");

  const toggleTool = (name: string, on: boolean) => {
    const next = new Set(allowed);
    if (on) next.add(name);
    else next.delete(name);
    setDraft({ ...draft, tools: tools.map((t) => t.name).filter((n) => next.has(n)) });
  };

  const save = async () => {
    setBusy(true);
    onError("");
    try {
      onSaved(await request<McpKey>("PUT", `/api/settings/mcp/keys/${entry.key.id}`, draft));
    } catch (e) {
      onError(errText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="mcp__key">
      <div className="security__row">
        <div style={{ flex: 1, minWidth: 200 }}>
          <b>{entry.key.name}</b>
          <div className="auth__hint">
            Reaches {entry.reachable.length} of {tools.length} tools
            {destructiveOn ? " · destructive tools on" : ""} · {entry.policy.rate_per_minute}/min
          </div>
        </div>
        <Button variant="secondary" onClick={onToggle} aria-expanded={open}>
          {open ? "Close" : "Tools…"}
        </Button>
      </div>
      {open && (
        <div className="mcp__panel">
          <label className="auth__check">
            <input
              type="checkbox"
              checked={draft.allow_destructive}
              onChange={(e) => setDraft({ ...draft, allow_destructive: e.target.checked })}
            />
            <span>
              Allow destructive tools
              <span className="auth__hint" style={{ display: "block" }}>
                Every call is written to the audit log with this key&apos;s name. Starting a print still needs the
                assistant to confirm the plate is clear.
              </span>
            </span>
          </label>
          <label className="mcp__rate">
            <span>Requests per minute</span>
            <input
              type="number"
              className="auth__input"
              min={1}
              max={600}
              value={draft.rate_per_minute}
              onChange={(e) => setDraft({ ...draft, rate_per_minute: Math.max(1, Number(e.target.value) || 1) })}
            />
          </label>
          {(["read", "write", "destructive"] as McpToolClass[]).map((cls) => (
            <fieldset key={cls} className="mcp__group">
              <legend className="auth__cap">{CLASS_LABEL[cls]}</legend>
              {tools
                .filter((t) => t.class === cls)
                .map((t) => {
                  const locked = cls === "destructive" && !draft.allow_destructive;
                  return (
                    <label key={t.name} className={`auth__check mcp__tool${locked ? " mcp__tool--off" : ""}`}>
                      <input
                        type="checkbox"
                        disabled={locked}
                        checked={allowed.has(t.name)}
                        onChange={(e) => toggleTool(t.name, e.target.checked)}
                      />
                      <span>
                        <span className="auth__mono">{t.name}</span>
                        <span className="auth__hint" style={{ display: "block" }}>
                          {t.description}
                        </span>
                      </span>
                    </label>
                  );
                })}
            </fieldset>
          ))}
          <div style={{ display: "flex", gap: "var(--fw-space-2)", flexWrap: "wrap" }}>
            <Button variant="primary" disabled={!dirty || busy} onClick={() => void save()}>
              Save
            </Button>
            <Button
              variant="secondary"
              disabled={busy}
              onClick={() => setDraft({ tools: null, allow_destructive: false, rate_per_minute: 60 })}
            >
              Reset to read-only
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}
