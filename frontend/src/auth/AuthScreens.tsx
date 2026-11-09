import { useState, type FormEvent, type ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, type ServerInfo } from "../api/client";
import { IS_TAURI, WindowControls } from "../shell/TopBar";

const MIN_PASSWORD = 10;
const CODE_RE = /^[A-Z0-9]{4}-[A-Z0-9]{4}$/;

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Window frame + centred column shared by every pre-sign-in view. */
export function AuthShell({ label, crumb, children }: { label: string; crumb: string; children?: ReactNode }) {
  const win = IS_TAURI ? getCurrentWindow() : null;
  return (
    <div className="auth">
      <div className="auth__frame" onDragStart={(e) => e.preventDefault()}>
        <div
          className="auth__brand"
          data-tauri-drag-region
          onMouseDown={
            win
              ? (e) => {
                  if (e.button !== 0 || e.detail === 2) return;
                  e.preventDefault();
                  void win.startDragging();
                }
              : undefined
          }
          onDoubleClick={win ? () => void win.toggleMaximize() : undefined}
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true" style={{ color: "var(--fw-accent)" }}>
            <rect x="3" y="3" width="18" height="18" />
            <path d="M3 9h18M9 9v12" />
          </svg>
          <span className="auth__brand-name">FOREMAN</span>
          <span className="auth__hint" style={{ marginLeft: "var(--fw-space-2)" }}>{label}</span>
        </div>
        {IS_TAURI && <WindowControls />}
      </div>
      <div className="auth__body">
        <main className="auth__main">
          <div className="auth__cap">{crumb}</div>
          {children}
        </main>
      </div>
    </div>
  );
}

/** Format typed input as XXXX-XXXX (uppercase, dash inserted). */
function formatCode(raw: string): string {
  const s = raw.toUpperCase().replace(/[^A-Z0-9]/g, "").slice(0, 8);
  return s.length > 4 ? `${s.slice(0, 4)}-${s.slice(4)}` : s;
}

export function SetupView({ info, onDone }: { info: ServerInfo; onDone: (username: string) => void }) {
  const addr = window.location.host;
  const [code, setCode] = useState("");
  const [username, setUsername] = useState("admin");
  const [pw, setPw] = useState("");
  const [pw2, setPw2] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const pwOk = pw.length >= MIN_PASSWORD;
  const ready = CODE_RE.test(code) && username.trim() !== "" && pwOk && pw === pw2;

  let pwHint = "At least 10 characters. A passphrase is fine.";
  let pwHintClass = "auth__hint";
  if (pw.length > 0) {
    pwHint = pwOk ? "✓ Long enough" : `${MIN_PASSWORD - pw.length} more characters`;
    pwHintClass = pwOk ? "auth__hint auth__hint--ok" : "auth__hint auth__hint--warn";
  }
  const pw2Hint = pw2.length === 0 ? "\u00a0" : pw2 === pw ? "✓ Matches" : "Does not match yet";
  const pw2HintClass = pw2 === pw ? "auth__hint auth__hint--ok" : "auth__hint auth__hint--warn";

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!ready || busy) return;
    setBusy(true);
    setError("");
    try {
      const res = await api.setupAdmin({ code, username: username.trim(), password: pw, remember: true });
      onDone(res.username);
    } catch (err) {
      setError(errText(err));
      setBusy(false);
    }
  };

  return (
    <AuthShell label={addr} crumb="Claim the server">
      <form className="auth__form" onSubmit={(e) => void submit(e)}>
        <div className="auth__server">
          <div style={{ flex: 1 }}>
            <b>{info.name}</b>
            <div className="auth__hint auth__mono">
              {addr} · Foreman {info.version} · new server
            </div>
          </div>
        </div>
        <div>
          <h1 className="auth__title">Set up the admin account</h1>
          <p className="auth__lead">
            This server has no admin yet. To prove you own it, enter the setup code it printed when it started.
          </p>
        </div>
        <div className="auth__note">
          Find it in the server's log: <span className="auth__code">journalctl -u foreman-server | grep "setup code"</span>
          <br />
          <span className="auth__hint">
            The code works once and expires after 24 h. Restarting the server prints a new one.
          </span>
        </div>
        <label className="auth__field">
          <span>Setup code</span>
          <input
            type="text"
            className="auth__input auth__input--code"
            value={code}
            onChange={(e) => setCode(formatCode(e.target.value))}
            placeholder="XXXX-XXXX"
            autoComplete="one-time-code"
            spellCheck={false}
            autoFocus
          />
        </label>
        <label className="auth__field">
          <span>Username</span>
          <input
            type="text"
            className="auth__input"
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            autoComplete="username"
          />
        </label>
        <label className="auth__field">
          <span>Password</span>
          <input
            type="password"
            className="auth__input"
            value={pw}
            onChange={(e) => setPw(e.target.value)}
            autoComplete="new-password"
          />
          <span className={pwHintClass}>{pwHint}</span>
        </label>
        <label className="auth__field">
          <span>Repeat password</span>
          <input
            type="password"
            className="auth__input"
            value={pw2}
            onChange={(e) => setPw2(e.target.value)}
            autoComplete="new-password"
          />
          <span className={pw2HintClass}>{pw2Hint}</span>
        </label>
        {error && (
          <div role="alert" className="auth__alert">
            {error}
          </div>
        )}
        <div className="auth__actions">
          <button type="submit" className="auth__btn auth__btn--pri" disabled={!ready || busy}>
            {busy ? "Creating…" : "Create admin and sign in"}
          </button>
        </div>
      </form>
    </AuthShell>
  );
}

export function LoginView({ info, onDone }: { info: ServerInfo; onDone: (username: string) => void }) {
  const addr = window.location.host;
  const [username, setUsername] = useState("");
  const [pw, setPw] = useState("");
  const [showPw, setShowPw] = useState(false);
  const [remember, setRemember] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (busy) return;
    if (!pw) {
      setError("Enter your password.");
      return;
    }
    setBusy(true);
    setError("");
    try {
      const res = await api.login({ username: username.trim(), password: pw, remember });
      onDone(res.username);
    } catch (err) {
      setError(errText(err));
      setBusy(false);
    }
  };

  return (
    <AuthShell label={addr} crumb="Sign in">
      <form className="auth__form" onSubmit={(e) => void submit(e)}>
        <div className="auth__server">
          <span className="auth__dot" aria-hidden="true" />
          <div style={{ flex: 1 }}>
            <b>{info.name}</b>
            <div className="auth__hint auth__mono">
              {addr} · Foreman {info.version}
            </div>
          </div>
        </div>
        <div>
          <h1 className="auth__title">Sign in</h1>
        </div>
        <label className="auth__field">
          <span>Username</span>
          <input
            type="text"
            className="auth__input"
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            autoComplete="username"
            autoFocus
          />
        </label>
        <label className="auth__field">
          <span>Password</span>
          <div className="auth__pw-row">
            <input
              type={showPw ? "text" : "password"}
              className="auth__input"
              value={pw}
              onChange={(e) => setPw(e.target.value)}
              autoComplete="current-password"
              aria-invalid={error ? true : undefined}
            />
            <button type="button" className="auth__btn auth__btn--sm" onClick={() => setShowPw((v) => !v)}>
              {showPw ? "Hide" : "Show"}
            </button>
          </div>
        </label>
        {error && (
          <div role="alert" className="auth__alert">
            {error}
          </div>
        )}
        <label className="auth__check">
          <input type="checkbox" checked={remember} onChange={(e) => setRemember(e.target.checked)} />
          <span>Keep me signed in on this device</span>
        </label>
        <div className="auth__actions">
          <button type="submit" className="auth__btn auth__btn--pri" disabled={busy}>
            {busy ? "Signing in…" : "Sign in"}
          </button>
          <span className="auth__hint">
            Forgot it? On the server run <span className="auth__mono">foreman-server reset-admin</span>
          </span>
        </div>
      </form>
    </AuthShell>
  );
}
