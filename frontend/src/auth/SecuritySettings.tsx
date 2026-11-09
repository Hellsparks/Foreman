import { useEffect, useState } from "react";
import { api, type ApiKey, type NetworkSettings } from "../api/client";
import Button from "../ui/Button";
import { useAuth } from "./AuthGate";
import { useRoute } from "../desktop/route";

/** SQLite `YYYY-MM-DD HH:MM:SS` (UTC) → local time. */
function fmtTime(ts: string | null): string {
  if (ts == null) return "never";
  const d = new Date(ts.includes("T") ? ts : `${ts.replace(" ", "T")}Z`);
  return Number.isNaN(d.getTime()) ? ts : d.toLocaleString();
}

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Settings → Security: session, API keys, LAN access (desktop only). */
export default function SecuritySettings() {
  const { username, logout } = useAuth();
  const [keys, setKeys] = useState<ApiKey[]>([]);
  const [name, setName] = useState("");
  const [secret, setSecret] = useState<{ name: string; value: string } | null>(null);
  const [copied, setCopied] = useState(false);
  const [net, setNet] = useState<NetworkSettings | null>(null);
  const [error, setError] = useState("");
  const [serverName, setServerName] = useState("");
  const [savedName, setSavedName] = useState("");
  const route = useRoute();

  useEffect(() => {
    api.listApiKeys().then(setKeys).catch((e) => setError(errText(e)));
    api.getNetwork().then(setNet).catch(() => {});
    api
      .serverInfo()
      .then((i) => {
        setServerName(i.name);
        setSavedName(i.name);
      })
      .catch(() => {});
  }, []);

  const rename = async () => {
    setError("");
    try {
      const res = await api.renameServer(serverName.trim());
      setServerName(res.name);
      setSavedName(res.name);
    } catch (e) {
      setError(errText(e));
    }
  };

  const create = async () => {
    const n = name.trim();
    if (!n) return;
    setError("");
    try {
      const res = await api.createApiKey(n);
      setKeys((prev) => [res.key, ...prev]);
      setSecret({ name: res.key.name, value: res.secret });
      setCopied(false);
      setName("");
    } catch (e) {
      setError(errText(e));
    }
  };

  const revoke = async (k: ApiKey) => {
    if (!window.confirm(`Revoke the API key "${k.name}"? Anything using it stops working.`)) return;
    setError("");
    try {
      await api.revokeApiKey(k.id);
      setKeys((prev) => prev.filter((x) => x.id !== k.id));
    } catch (e) {
      setError(errText(e));
    }
  };

  const toggleLan = async (allow: boolean) => {
    setError("");
    try {
      setNet(await api.setNetwork(allow));
    } catch (e) {
      setError(errText(e));
    }
  };

  const copy = async () => {
    if (!secret) return;
    try {
      await navigator.clipboard.writeText(secret.value);
      setCopied(true);
    } catch {
      /* clipboard blocked — the key is selectable */
    }
  };

  return (
    <section aria-labelledby="security-heading" style={{ maxWidth: 720, marginTop: "var(--fw-space-8)" }}>
      <h2
        id="security-heading"
        style={{ font: "500 18px/1.3 var(--fw-sans)", margin: "0 0 var(--fw-space-4)", letterSpacing: "-0.02em" }}
      >
        Security
      </h2>

      <div className="security__row">
        <span style={{ flex: 1 }}>
          Signed in as <b>{username}</b>
        </span>
        <Button variant="secondary" onClick={() => void logout()}>
          Log out
        </Button>
      </div>

      {route && (
        <div className="security__row" style={{ marginTop: 2 }}>
          <span style={{ flex: 1 }}>
            Connected to <b>{route.serverName}</b>{" "}
            <span className="auth__hint auth__mono">
              {route.address}
              {route.via ? ` · via ${route.via}` : ""}
            </span>
          </span>
          <Button variant="secondary" onClick={route.switchServer}>
            Switch server
          </Button>
        </div>
      )}

      <h3 className="auth__cap" style={{ margin: "var(--fw-space-6) 0 var(--fw-space-2)", fontWeight: "normal" }}>
        Server name
      </h3>
      <form
        style={{ display: "flex", gap: "var(--fw-space-2)" }}
        onSubmit={(e) => {
          e.preventDefault();
          void rename();
        }}
      >
        <input
          type="text"
          className="auth__input"
          style={{ flex: 1 }}
          aria-label="Server name"
          value={serverName}
          maxLength={64}
          onChange={(e) => setServerName(e.target.value)}
        />
        <Button type="submit" variant="secondary" disabled={!serverName.trim() || serverName.trim() === savedName}>
          Rename
        </Button>
      </form>
      <p className="auth__hint" style={{ margin: "var(--fw-space-2) 0 0" }}>
        Shown in the app's server list and announced on the LAN.
      </p>

      <h3 className="auth__cap" style={{ margin: "var(--fw-space-6) 0 var(--fw-space-2)", fontWeight: "normal" }}>
        API keys
      </h3>
      <p className="auth__hint" style={{ margin: "0 0 var(--fw-space-3)" }}>
        For slicers, MCP and scripts. Send it as the <span className="auth__mono">X-Api-Key</span> header.
      </p>

      {secret && (
        <div className="security__secret" role="status" style={{ marginBottom: "var(--fw-space-3)" }}>
          <span>
            New key <b>{secret.name}</b> — copy it now, it is only shown once.
          </span>
          <code>{secret.value}</code>
          <div style={{ display: "flex", gap: "var(--fw-space-2)" }}>
            <Button variant="small" onClick={() => void copy()}>
              {copied ? "Copied" : "Copy"}
            </Button>
            <Button variant="small" onClick={() => setSecret(null)}>
              Done
            </Button>
          </div>
        </div>
      )}

      <form
        style={{ display: "flex", gap: "var(--fw-space-2)", marginBottom: "var(--fw-space-3)" }}
        onSubmit={(e) => {
          e.preventDefault();
          void create();
        }}
      >
        <input
          type="text"
          className="auth__input"
          style={{ flex: 1 }}
          placeholder="Key name, e.g. OrcaSlicer"
          aria-label="New API key name"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
        <Button type="submit" variant="primary" disabled={!name.trim()}>
          Create key
        </Button>
      </form>

      {keys.length === 0 ? (
        <p className="auth__hint">No API keys yet.</p>
      ) : (
        <div className="security__list">
          {keys.map((k) => (
            <div key={k.id} className="security__row">
              <div style={{ flex: 1, minWidth: 200 }}>
                <b>{k.name}</b>
                <div className="auth__hint auth__mono">
                  {k.prefix}… · created {fmtTime(k.created_at)} · last used {fmtTime(k.last_used_at)}
                </div>
              </div>
              <Button variant="danger" onClick={() => void revoke(k)}>
                Revoke
              </Button>
            </div>
          ))}
        </div>
      )}

      {net?.desktop_mode && (
        <>
          <h3 className="auth__cap" style={{ margin: "var(--fw-space-6) 0 var(--fw-space-2)", fontWeight: "normal" }}>
            Network
          </h3>
          <label className="auth__check security__row">
            <input type="checkbox" checked={net.allow_lan} onChange={(e) => void toggleLan(e.target.checked)} />
            <span style={{ flex: 1 }}>
              Let other devices connect
              <span className="auth__hint" style={{ display: "block" }}>
                Listening on <span className="auth__mono">{net.bound_host}</span>.
                {net.restart_required && " Restart Foreman to apply."}
              </span>
            </span>
          </label>
        </>
      )}

      {error && (
        <div role="alert" className="auth__alert" style={{ marginTop: "var(--fw-space-3)" }}>
          {error}
        </div>
      )}
    </section>
  );
}
