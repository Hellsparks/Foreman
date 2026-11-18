/**
 * Settings → Integrations → Bambu Cloud (doc 07 §B): many accounts, each signed in on
 * its own. Sign-in: email + password, then a 6-digit code if Bambu asks for one (email
 * or authenticator app, same form), or paste an existing access token. The browser never
 * sees the token, only "connected until <date>".
 */
import { useCallback, useEffect, useState, type FormEvent } from "react";
import { bambuApi, type BambuAccountView, type CloudDevice, type Region } from "../api/bambu";
import { Button, StatusChip } from "../ui";
import { canListDevices, codePrompt, isSixDigits, removeWarning, stateChip } from "./bambuLogic";
import "./BambuCloud.css";

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

type Method = "password" | "token";

export default function BambuCloudSettings() {
  const [accounts, setAccounts] = useState<BambuAccountView[] | null>(null);
  const [error, setError] = useState("");
  const [adding, setAdding] = useState(false);

  const load = useCallback(async () => {
    try {
      setAccounts(await bambuApi.list());
      setError("");
    } catch (e) {
      setError(errText(e));
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load]);

  const replace = (a: BambuAccountView) =>
    setAccounts((prev) => (prev ?? []).map((x) => (x.id === a.id ? a : x)));

  return (
    <div className="bc">
      <h3 className="auth__cap bc__cap">Bambu Cloud</h3>
      <p className="auth__hint bc__intro">
        Sign in to one or more Bambu Lab accounts. Each account is independent: one failing or expiring never
        affects another. Your password is used once to get a token and is never stored.
      </p>

      {error && <p className="auth__hint bc__err" role="alert">{error}</p>}
      {accounts === null && !error && <p className="auth__hint">Loading…</p>}
      {accounts?.length === 0 && !adding && <p className="auth__hint">No Bambu accounts yet.</p>}

      <div className="bc__list">
        {accounts?.map((a) => (
          <AccountCard
            key={a.id}
            account={a}
            onChange={replace}
            onRemoved={() => setAccounts((prev) => (prev ?? []).filter((x) => x.id !== a.id))}
            onReload={load}
          />
        ))}
      </div>

      {adding ? (
        <AddAccount
          onCancel={() => setAdding(false)}
          onAdded={() => {
            setAdding(false);
            void load();
          }}
        />
      ) : (
        <Button variant="secondary" onClick={() => setAdding(true)} style={{ marginTop: "var(--fw-space-3)" }}>
          Add account…
        </Button>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Add account: who (label, region, email), then how (password or token)
// ---------------------------------------------------------------------------

function AddAccount({ onCancel, onAdded }: { onCancel: () => void; onAdded: () => void }) {
  const [label, setLabel] = useState("");
  const [region, setRegion] = useState<Region>("global");
  const [email, setEmail] = useState("");
  const [method, setMethod] = useState<Method>("password");
  const [secret, setSecret] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setErr("");
    try {
      const a = await bambuApi.create({ label, region, email });
      // The account exists now; a failed sign-in shows on its card, not here.
      try {
        await (method === "password" ? bambuApi.signin(a.id, secret) : bambuApi.token(a.id, secret));
      } catch {
        /* recorded on the account */
      }
      setSecret("");
      onAdded();
    } catch (e2) {
      setErr(errText(e2));
      setBusy(false);
    }
  };

  return (
    <form className="bc__card bc__form" onSubmit={(e) => void submit(e)} autoComplete="off">
      <div className="bc__card-title">Add Bambu account</div>
      <div className="bc__grid">
        <label className="auth__field">
          <span>Label</span>
          <input className="auth__input" value={label} placeholder="e.g. Workshop" onChange={(e) => setLabel(e.target.value)} />
        </label>
        <label className="auth__field">
          <span>Region</span>
          <select className="auth__input" value={region} onChange={(e) => setRegion(e.target.value as Region)}>
            <option value="global">Global (bambulab.com)</option>
            <option value="china">China (bambulab.cn)</option>
          </select>
        </label>
        <label className="auth__field bc__wide">
          <span>Email</span>
          <input
            className="auth__input"
            type="email"
            required
            value={email}
            autoComplete="username"
            onChange={(e) => setEmail(e.target.value)}
          />
        </label>
      </div>
      <SecretFields method={method} setMethod={setMethod} secret={secret} setSecret={setSecret} />
      {err && <p className="auth__hint bc__err" role="alert">{err}</p>}
      <div className="bc__actions">
        <Button type="submit" variant="primary" disabled={busy || email.trim() === "" || secret === ""}>
          {busy ? "Signing in…" : method === "password" ? "Sign in" : "Check token"}
        </Button>
        <Button onClick={onCancel} disabled={busy}>Cancel</Button>
      </div>
    </form>
  );
}

function SecretFields({
  method,
  setMethod,
  secret,
  setSecret,
}: {
  method: Method;
  setMethod: (m: Method) => void;
  secret: string;
  setSecret: (s: string) => void;
}) {
  return (
    <>
      <fieldset className="bc__method">
        <legend className="ui-sr">Sign-in method</legend>
        <label className="auth__check">
          <input type="radio" checked={method === "password"} onChange={() => { setMethod("password"); setSecret(""); }} />
          <span>Password</span>
        </label>
        <label className="auth__check">
          <input type="radio" checked={method === "token"} onChange={() => { setMethod("token"); setSecret(""); }} />
          <span>Paste an access token</span>
        </label>
      </fieldset>
      {method === "password" ? (
        <label className="auth__field">
          <span>Password</span>
          <input
            className="auth__input"
            type="password"
            autoComplete="current-password"
            value={secret}
            onChange={(e) => setSecret(e.target.value)}
          />
          <span className="auth__hint">
            Signed up with Google or Apple? Set a password in the Bambu app first, or paste a token instead.
          </span>
        </label>
      ) : (
        <label className="auth__field">
          <span>Access token</span>
          <input
            className="auth__input auth__mono"
            type="password"
            autoComplete="off"
            spellCheck={false}
            value={secret}
            onChange={(e) => setSecret(e.target.value)}
          />
          <span className="auth__hint">Checked against your Bambu profile before it is saved.</span>
        </label>
      )}
    </>
  );
}

// ---------------------------------------------------------------------------
// One account
// ---------------------------------------------------------------------------

function AccountCard({
  account: a,
  onChange,
  onRemoved,
  onReload,
}: {
  account: BambuAccountView;
  onChange: (a: BambuAccountView) => void;
  onRemoved: () => void;
  onReload: () => Promise<void>;
}) {
  const chip = stateChip(a);
  const [reauth, setReauth] = useState(false);
  const [removing, setRemoving] = useState(false);
  // Bumped after each successful sign-in so the device list is fetched again.
  const [signins, setSignins] = useState(0);
  const signedIn = (next: BambuAccountView) => {
    setSignins((n) => n + 1);
    onChange(next);
  };
  const live = canListDevices(a);

  return (
    <div className="bc__card" aria-label={`Bambu account ${a.label}`}>
      <div className="bc__head">
        <div className="bc__who">
          <div className="bc__card-title">{a.label}</div>
          <div className="auth__hint">
            <span className="bc__region">{a.region === "china" ? "China" : "Global"}</span> · {a.email}
          </div>
        </div>
        <div className="bc__state">
          <StatusChip tone={chip.tone} symbol={chip.symbol} word={chip.word} />
          {chip.detail && <div className={`auth__hint bc__detail${chip.tone === "bad" ? " bc__err" : ""}`}>{chip.detail}</div>}
        </div>
      </div>

      {a.state === "code_required" && <CodeForm account={a} onDone={signedIn} onRestart={() => setReauth(true)} />}

      {live && <Devices key={signins} account={a} onReload={onReload} />}

      {reauth && (
        <SigninForm
          account={a}
          onDone={(next) => {
            setReauth(false);
            signedIn(next);
          }}
          onFailed={() => void onReload()}
          onCancel={() => setReauth(false)}
        />
      )}

      {removing ? (
        <RemoveConfirm account={a} onCancel={() => setRemoving(false)} onRemoved={onRemoved} />
      ) : (
        !reauth && (
          <div className="bc__actions">
            <Button variant="small" onClick={() => setReauth(true)}>
              {a.has_token ? "Re-authenticate" : "Sign in"}
            </Button>
            <Button variant="small" onClick={() => setRemoving(true)}>Remove…</Button>
          </div>
        )
      )}
    </div>
  );
}

function SigninForm({
  account,
  onDone,
  onFailed,
  onCancel,
}: {
  account: BambuAccountView;
  onDone: (a: BambuAccountView) => void;
  onFailed: () => void;
  onCancel: () => void;
}) {
  const [method, setMethod] = useState<Method>("password");
  const [secret, setSecret] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setErr("");
    try {
      const next = method === "password" ? await bambuApi.signin(account.id, secret) : await bambuApi.token(account.id, secret);
      setSecret("");
      onDone(next);
    } catch (e2) {
      // Never retried: show the error and wait for the user.
      setErr(errText(e2));
      setBusy(false);
      onFailed();
    }
  };

  return (
    <form className="bc__form bc__inner" onSubmit={(e) => void submit(e)} autoComplete="off">
      <input type="email" hidden readOnly value={account.email} autoComplete="username" />
      <SecretFields method={method} setMethod={setMethod} secret={secret} setSecret={setSecret} />
      {err && <p className="auth__hint bc__err" role="alert">{err}</p>}
      <div className="bc__actions">
        <Button type="submit" variant="primary" disabled={busy || secret === ""}>
          {busy ? "Signing in…" : method === "password" ? "Sign in" : "Check token"}
        </Button>
        <Button onClick={onCancel} disabled={busy}>Cancel</Button>
      </div>
    </form>
  );
}

function CodeForm({
  account,
  onDone,
  onRestart,
}: {
  account: BambuAccountView;
  onDone: (a: BambuAccountView) => void;
  onRestart: () => void;
}) {
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setErr("");
    try {
      onDone(await bambuApi.code(account.id, code.trim()));
    } catch (e2) {
      setErr(errText(e2));
      setBusy(false);
    }
  };

  return (
    <form className="bc__form bc__inner" onSubmit={(e) => void submit(e)} autoComplete="off">
      <label className="auth__field">
        <span>Verification code</span>
        <input
          className="auth__input auth__input--code"
          inputMode="numeric"
          autoComplete="one-time-code"
          maxLength={6}
          autoFocus
          value={code}
          onChange={(e) => setCode(e.target.value.replace(/\D/g, ""))}
        />
        <span className="auth__hint">{codePrompt(account.code_channel)}</span>
      </label>
      {err && <p className="auth__hint bc__err" role="alert">{err}</p>}
      <div className="bc__actions">
        <Button type="submit" variant="primary" disabled={busy || !isSixDigits(code)}>
          {busy ? "Checking…" : "Verify"}
        </Button>
        <Button variant="small" onClick={onRestart} disabled={busy}>Start over</Button>
      </div>
    </form>
  );
}

function Devices({ account, onReload }: { account: BambuAccountView; onReload: () => Promise<void> }) {
  const [devices, setDevices] = useState<CloudDevice[] | null>(null);
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);

  const fetchList = useCallback(async () => {
    setBusy(true);
    setErr("");
    try {
      setDevices(await bambuApi.devices(account.id));
    } catch (e) {
      setErr(errText(e));
      setDevices(null);
    } finally {
      setBusy(false);
    }
  }, [account.id]);

  // Once per mount; the card remounts this after each successful sign-in. The account
  // row is reloaded afterwards so its state reflects this call (ok or token rejected).
  useEffect(() => {
    void fetchList().then(onReload);
  }, [fetchList, onReload]);

  return (
    <div className="bc__devices">
      <div className="bc__devices-head">
        <span className="auth__cap">Devices</span>
        <Button
          variant="small"
          disabled={busy}
          onClick={() => {
            void fetchList().then(onReload);
          }}
        >
          {busy ? "Loading…" : "Refresh"}
        </Button>
      </div>
      {err && <p className="auth__hint bc__err" role="alert">{err}</p>}
      {devices && devices.length === 0 && <p className="auth__hint">No printers are bound to this account.</p>}
      {devices && devices.length > 0 && (
        <table className="bc__table">
          <thead>
            <tr>
              <th scope="col">Name</th>
              <th scope="col">Model</th>
              <th scope="col">Serial</th>
              <th scope="col">Online</th>
            </tr>
          </thead>
          <tbody>
            {devices.map((d) => (
              <tr key={d.dev_id}>
                <td>{d.name || "—"}</td>
                <td>{d.dev_product_name ?? d.dev_model_name ?? "—"}</td>
                <td className="auth__mono">{d.dev_id}</td>
                <td>
                  {d.online ? (
                    <StatusChip tone="ok" symbol="●" word="Online" />
                  ) : (
                    <StatusChip tone="off" symbol="○" word="Offline" />
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

function RemoveConfirm({
  account,
  onCancel,
  onRemoved,
}: {
  account: BambuAccountView;
  onCancel: () => void;
  onRemoved: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const remove = async () => {
    setBusy(true);
    setErr("");
    try {
      await bambuApi.remove(account.id);
      onRemoved();
    } catch (e) {
      setErr(errText(e));
      setBusy(false);
    }
  };
  return (
    <div className="bc__inner bc__remove" role="group" aria-label={`Remove ${account.label}`}>
      <div className="bc__remove-text">
        <strong>Remove “{account.label}”?</strong>
        <div className="auth__hint">{removeWarning(account)} The stored token is deleted.</div>
      </div>
      {err && <p className="auth__hint bc__err" role="alert">{err}</p>}
      <div className="bc__actions">
        <Button variant="danger" disabled={busy} onClick={() => void remove()}>
          {busy ? "Removing…" : "Remove account"}
        </Button>
        <Button onClick={onCancel} disabled={busy}>Cancel</Button>
      </div>
    </div>
  );
}
