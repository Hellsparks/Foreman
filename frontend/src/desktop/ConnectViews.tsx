import { useEffect, useState } from "react";
import { AuthShell } from "../auth/AuthScreens";
import { addressKind, reportedKind, sameAddress, timeAgo, withPort } from "./addresses";
import { desktop, type Found, type LocalConfig, type Probe, type SavedServer } from "./bridge";
import type { AddrState } from "./failover";

type Tone = "ok" | "warn" | "bad";

const LOCAL_ADDRESS = "127.0.0.1:9090";

function Notice({ tone, title, body }: { tone: Tone; title: string; body?: string }) {
  return (
    <div className={`connect__notice connect--${tone}`} role={tone === "bad" ? "alert" : "status"}>
      <div className="connect__notice-title">{title}</div>
      {body && <div className="connect__notice-body">{body}</div>}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Welcome
// ---------------------------------------------------------------------------

export function WelcomeView({
  saved,
  onOpen,
  onEdit,
  onRemote,
  onLocal,
}: {
  saved: SavedServer[];
  onOpen: (id: string) => void;
  onEdit: (id: string) => void;
  onRemote: () => void;
  onLocal: () => void;
}) {
  const [up, setUp] = useState<Record<string, boolean>>({});
  useEffect(() => {
    let live = true;
    for (const s of saved) {
      void desktop
        .check(s.addresses[0] ?? "")
        .then((id) => live && setUp((u) => ({ ...u, [s.id]: id === s.id })))
        .catch(() => {});
    }
    return () => {
      live = false;
    };
  }, [saved]);

  return (
    <AuthShell label="Not connected" crumb="Step 1 of 2 · Choose a server">
      <div className="auth__form" style={{ gap: "var(--fw-space-6)" }}>
        <div>
          <h1 className="auth__title">Where is your Foreman server?</h1>
          <p className="auth__lead">
            The server runs the queue, logs jobs and talks to the printers. This app is a window onto it.
          </p>
        </div>

        {saved.length > 0 && (
          <section aria-label="Saved servers" className="connect__stack">
            <div className="auth__cap">Recent servers</div>
            {saved.map((s) => {
              const backups = s.addresses.slice(1);
              return (
                <div key={s.id} className="connect__row">
                  <span
                    className="auth__dot"
                    style={{ background: up[s.id] ? "var(--fw-success)" : "var(--fw-muted)" }}
                    aria-label={up[s.id] ? "answering" : "not answering"}
                  />
                  <div className="connect__grow">
                    <b>{s.name}</b>
                    <div className="auth__hint auth__mono">
                      {s.addresses[0]} · {timeAgo(s.last_used)}
                    </div>
                    <div className="auth__hint">
                      {backups.length
                        ? `+ ${backups.length} backup route${backups.length === 1 ? "" : "s"}: ${backups.map(addressKind).join(", ")}`
                        : "no backup routes"}
                    </div>
                  </div>
                  <button type="button" className="auth__btn auth__btn--sm" onClick={() => onEdit(s.id)}>
                    Edit
                  </button>
                  <button type="button" className="auth__btn auth__btn--sm auth__btn--pri" onClick={() => onOpen(s.id)}>
                    Connect
                  </button>
                </div>
              );
            })}
          </section>
        )}

        <div className="connect__stack" style={{ gap: "var(--fw-space-3)" }}>
          <button type="button" className="connect__choice" onClick={onRemote}>
            <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden="true" className="connect__choice-icon">
              <rect x="3" y="4" width="18" height="6" />
              <rect x="3" y="14" width="18" height="6" />
              <path d="M7 7h.01M7 17h.01" />
            </svg>
            <span className="connect__grow">
              <b className="connect__choice-title">Another computer</b>
              <span className="connect__badge">Recommended</span>
              <br />
              <span className="connect__muted">
                A Pi, NAS or PC running foreman-server. Keeps printing, queueing and logging when this app is closed.
              </span>
            </span>
          </button>
          <button type="button" className="connect__choice" onClick={onLocal}>
            <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden="true" className="connect__choice-icon">
              <rect x="3" y="4" width="18" height="12" />
              <path d="M8 20h8M12 16v4" />
            </svg>
            <span className="connect__grow">
              <b className="connect__choice-title">This computer</b>
              <br />
              <span className="connect__muted">
                Runs the server inside this app. Good for trying Foreman. Queue and job logging stop when you close it.
              </span>
            </span>
          </button>
        </div>
      </div>
    </AuthShell>
  );
}

// ---------------------------------------------------------------------------
// Local (Mode A)
// ---------------------------------------------------------------------------

export function LocalView({
  onBack,
  onStart,
  error,
}: {
  onBack: () => void;
  onStart: (lan: boolean) => Promise<void>;
  error: string;
}) {
  const [cfg, setCfg] = useState<LocalConfig | null>(null);
  const [lan, setLan] = useState(false);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    void desktop.localConfig().then(setCfg).catch(() => {});
  }, []);

  return (
    <AuthShell label="Not connected" crumb="Step 2 of 2 · This computer">
      <div className="auth__form">
        <div>
          <h1 className="auth__title">Run the server here</h1>
          <p className="auth__lead">Foreman and the built-in Spoolman start with this app.</p>
        </div>
        <section className="connect__panel">
          <div className="connect__kv">
            <span>Foreman</span>
            <span className="auth__mono">port {cfg?.port ?? "…"}</span>
          </div>
          {cfg?.spoolman_port != null && (
            <div className="connect__kv">
              <span>Spoolman (built in)</span>
              <span className="auth__mono">port {cfg.spoolman_port}</span>
            </div>
          )}
          <div className="connect__kv">
            <span>Data folder</span>
            <span className="auth__mono" style={{ fontSize: 13 }}>{cfg?.data_dir ?? "…"}</span>
          </div>
        </section>
        <label className="auth__check connect__lan">
          <input type="checkbox" checked={lan} onChange={(e) => setLan(e.target.checked)} />
          <span>
            <b>Let other devices connect</b>
            <br />
            <span className="auth__hint">
              Phone, another PC, Tailscale. Needs an admin password; without it the server only answers this computer.
            </span>
          </span>
        </label>
        {error && <Notice tone="bad" title="✖ The built-in server did not start" body={error} />}
        <div className="auth__actions">
          <button type="button" className="auth__btn" onClick={onBack}>
            Back
          </button>
          <button
            type="button"
            className="auth__btn auth__btn--pri"
            disabled={busy}
            onClick={() => {
              setBusy(true);
              void onStart(lan).finally(() => setBusy(false));
            }}
          >
            {busy ? "Starting…" : lan ? "Start and set admin password" : "Start server here"}
          </button>
        </div>
      </div>
    </AuthShell>
  );
}

// ---------------------------------------------------------------------------
// Connect (Mode B)
// ---------------------------------------------------------------------------

interface Route {
  addr: string;
  kind: string;
  src: string;
}

interface Result {
  tone: Tone;
  title: string;
  body: string;
  probe: Probe | null;
}

function describe(p: Probe, saved: SavedServer[]): Result {
  switch (p.kind) {
    case "invalid":
      return { tone: "bad", title: "✖ Enter an address", body: "An IP address, a hostname, or a Tailscale name.", probe: p };
    case "spoolman":
      return {
        tone: "warn",
        title: "⚠ That is the Spoolman port",
        body: `Port 7912 is the built-in Spoolman API that Moonraker and slicers use. Foreman itself listens on 9090. Try ${p.suggestion}.`,
        probe: p,
      };
    case "moonraker":
      return {
        tone: "warn",
        title: "⚠ That is a printer, not a Foreman server",
        body: `${p.hostname ?? p.address} answers as Moonraker. Add it as a printer after you connect; here you need the machine that runs foreman-server.`,
        probe: p,
      };
    case "no_answer":
      return {
        tone: "bad",
        title: `✖ No answer from ${p.address}`,
        body: "Check the address, that foreman-server is running, and that port 9090 is open in the server firewall. On Tailscale, check both devices are online.",
        probe: p,
      };
    case "foreman": {
      const known = saved.find((s) => s.id === p.server_id);
      if (known && !known.addresses.some((a) => sameAddress(a, p.address))) {
        return {
          tone: "ok",
          title: `✓ This is ${known.name}, which you already have saved`,
          body: `Same server ID as the saved ${known.name}, just a different address (${addressKind(p.address)}). Add it as a backup route so the app can fall back to it.`,
          probe: p,
        };
      }
      if (p.needs_setup) {
        return {
          tone: "ok",
          title: `✓ Foreman ${p.version} found, not set up yet`,
          body: `${p.name} has no admin account. Next you set one up with the setup code from its log.`,
          probe: p,
        };
      }
      return { tone: "ok", title: `✓ Foreman ${p.version} found`, body: `${p.name} · sign-in required.`, probe: p };
    }
  }
}

export function ConnectView({
  saved,
  onBack,
  onContinue,
  onAddBackup,
  onOpenSaved,
}: {
  saved: SavedServer[];
  onBack: () => void;
  onContinue: (server: SavedServer) => void;
  onAddBackup: (serverId: string, address: string) => void;
  onOpenSaved: (serverId: string, address: string) => void;
}) {
  const [found, setFound] = useState<Found[]>([]);
  const [scanning, setScanning] = useState(false);
  const [addr, setAddr] = useState("");
  const [testing, setTesting] = useState(false);
  const [result, setResult] = useState<Result | null>(null);
  const [routes, setRoutes] = useState<Route[]>([]);
  const [newRoute, setNewRoute] = useState("");
  const [scanError, setScanError] = useState("");

  const scan = async () => {
    setScanning(true);
    setScanError("");
    const errors: string[] = [];
    try {
      // An unclaimed server only listens on 127.0.0.1 and isn't announced, so look here too.
      const [list, local] = await Promise.all([
        desktop.discover().catch((e) => {
          errors.push(`LAN scan: ${String(e)}`);
          return [] as Found[];
        }),
        desktop.probe(LOCAL_ADDRESS).catch((e) => {
          errors.push(`${LOCAL_ADDRESS}: ${String(e)}`);
          return null;
        }),
      ]);
      if (local?.kind === "foreman" && !list.some((f) => f.id === local.server_id)) {
        list.unshift({
          id: local.server_id,
          name: local.name,
          version: local.version,
          host: "This computer",
          needs_setup: local.needs_setup,
          addresses: [local.address],
        });
      }
      setFound(list);
    } finally {
      setScanError(errors.join(" · "));
      setScanning(false);
    }
  };

  useEffect(() => {
    void scan();
  }, []);

  const test = async (raw: string) => {
    setTesting(true);
    setResult(null);
    try {
      const p = await desktop.probe(raw);
      const r = describe(p, saved);
      setResult(r);
      if (p.kind === "foreman" || p.kind === "no_answer" || p.kind === "spoolman" || p.kind === "moonraker") {
        setAddr(p.address);
      }
      if (p.kind === "foreman") {
        const prev = saved.find((s) => s.id === p.server_id);
        const fromSaved = (prev?.addresses ?? []).map((a) => ({ addr: a, kind: addressKind(a), src: "saved" }));
        const reported = p.addresses.map((a) => ({
          addr: a.address,
          kind: reportedKind(a.kind),
          src: "reported by server",
        }));
        const list: Route[] = [];
        for (const r of [...fromSaved, ...reported]) {
          if (sameAddress(r.addr, p.address) || list.some((x) => sameAddress(x.addr, r.addr))) continue;
          if (r.kind === "This computer") continue;
          list.push(r);
        }
        setRoutes(list);
      }
    } finally {
      setTesting(false);
    }
  };

  const p = result?.probe;
  const isForeman = p?.kind === "foreman";
  const knownElsewhere =
    isForeman && saved.find((s) => s.id === p.server_id && !s.addresses.some((a) => sameAddress(a, p.address)));
  const canContinue = isForeman && !knownElsewhere;

  const move = (i: number) => {
    if (i === 0) return;
    setRoutes((r) => {
      const a = [...r];
      [a[i - 1], a[i]] = [a[i], a[i - 1]];
      return a;
    });
  };

  return (
    <AuthShell label="Not connected" crumb="Step 1 of 2 · Server address">
      <div className="auth__form">
        <div>
          <h1 className="auth__title">Connect to a server</h1>
          <p className="auth__lead">Pick one found on your network, or type its address.</p>
        </div>

        <section aria-label="Found on your network" className="connect__stack">
          <div className="connect__head">
            <span className="auth__cap connect__grow">Found on your network</span>
            <button type="button" className="auth__btn auth__btn--sm" onClick={() => void scan()} disabled={scanning}>
              {scanning ? "Scanning…" : "Scan again"}
            </button>
          </div>
          {!scanning && found.length === 0 && <p className="auth__hint">No Foreman servers answered on this network.</p>}
          {!scanning && scanError && <Notice tone="bad" title="✖ Scan failed" body={scanError} />}
          {!scanning &&
            found.map((f) => {
              const a = f.addresses[0] ?? f.host;
              const tone: Tone = f.needs_setup ? "warn" : "ok";
              return (
                <button
                  type="button"
                  key={f.id}
                  className={`connect__row connect__pick${addr && sameAddress(addr, a) ? " connect__pick--on" : ""}`}
                  onClick={() => {
                    setAddr(a);
                    void test(a);
                  }}
                >
                  <div className="connect__grow">
                    <b>{f.name}</b>
                    <div className="auth__hint auth__mono">
                      {a} · Foreman {f.version}
                      {f.needs_setup ? " · new" : ""}
                    </div>
                  </div>
                  <span className={`connect__tag connect--${tone}`}>{f.needs_setup ? "Not set up" : "Sign-in"}</span>
                </button>
              );
            })}
        </section>

        <form
          className="auth__field"
          onSubmit={(e) => {
            e.preventDefault();
            void test(addr);
          }}
        >
          <label className="auth__field">
            <span>Server address</span>
            <input
              type="text"
              className="auth__input auth__mono"
              value={addr}
              onChange={(e) => {
                setAddr(e.target.value);
                setResult(null);
              }}
              placeholder="10.0.0.20  ·  foreman.local  ·  my-pi.tail1234.ts.net:9090"
              spellCheck={false}
            />
            <span className="auth__hint">Port 9090 is added if you leave it out.</span>
          </label>
        </form>

        {result && <Notice tone={result.tone} title={result.title} body={result.body} />}

        {canContinue && (
          <section aria-label="Backup addresses" className="connect__panel">
            <div className="connect__panel-head auth__cap">
              Backup addresses · tried in order when the one above stops answering
            </div>
            {routes.map((r, i) => (
              <div key={r.addr} className="connect__list-row">
                <span className="auth__mono connect__num">{i + 2}.</span>
                <div className="connect__grow">
                  <span className="auth__mono" style={{ fontSize: 13 }}>{r.addr}</span>
                  <div className="auth__hint">
                    {r.kind} · {r.src}
                  </div>
                </div>
                <button type="button" className="auth__btn auth__btn--sm connect__icon-btn" aria-label="Move up" onClick={() => move(i)}>
                  ↑
                </button>
                <button
                  type="button"
                  className="auth__btn auth__btn--sm"
                  onClick={() => setRoutes((x) => x.filter((_, j) => j !== i))}
                >
                  Remove
                </button>
              </div>
            ))}
            <form
              className="connect__add"
              onSubmit={(e) => {
                e.preventDefault();
                const v = newRoute.trim();
                if (!v) return;
                const a = withPort(v);
                if (!routes.some((x) => sameAddress(x.addr, a)) && !(p && sameAddress(addr, a))) {
                  setRoutes((x) => [...x, { addr: a, kind: addressKind(a), src: "added by you · not tested yet" }]);
                }
                setNewRoute("");
              }}
            >
              <input
                type="text"
                className="auth__input auth__mono connect__add-input"
                value={newRoute}
                onChange={(e) => setNewRoute(e.target.value)}
                placeholder="Add address, e.g. 192.168.50.20 or vpn name"
                aria-label="Add backup address"
              />
              <button type="submit" className="auth__btn auth__btn--sm">
                Add
              </button>
            </form>
            <div className="auth__hint connect__pad">
              Same server, same sign-in: the app checks the server's ID on every address, so a different machine on a
              backup address is refused.
            </div>
          </section>
        )}

        {knownElsewhere && p?.kind === "foreman" && (
          <div className="auth__actions">
            <button type="button" className="auth__btn auth__btn--pri" onClick={() => onAddBackup(p.server_id, p.address)}>
              Add as backup route for {knownElsewhere.name}
            </button>
            <button type="button" className="auth__btn" onClick={() => onOpenSaved(p.server_id, p.address)}>
              Just connect
            </button>
          </div>
        )}

        <div className="auth__actions">
          <button type="button" className="auth__btn" onClick={onBack}>
            Back
          </button>
          <button type="button" className="auth__btn" onClick={() => void test(addr)} disabled={testing}>
            {testing ? "Testing…" : "Test"}
          </button>
          {canContinue && p?.kind === "foreman" && (
            <button
              type="button"
              className="auth__btn auth__btn--pri"
              onClick={() => {
                const prev = saved.find((s) => s.id === p.server_id);
                onContinue({
                  id: p.server_id,
                  name: prev?.name ?? p.name,
                  version: p.version,
                  addresses: [p.address, ...routes.map((r) => r.addr)],
                  token: prev?.token ?? null,
                  last_used: Date.now(),
                });
              }}
            >
              Continue
            </button>
          )}
        </div>
      </div>
    </AuthShell>
  );
}

// ---------------------------------------------------------------------------
// Edit a saved server
// ---------------------------------------------------------------------------

type TestState = "ok" | "down" | "other" | "none";

const TEST_LABEL: Record<TestState, [string, Tone | "muted"]> = {
  ok: ["✓ Same server", "ok"],
  down: ["✖ No answer", "bad"],
  other: ["✖ Different server", "bad"],
  none: ["not tested", "muted"],
};

export function EditView({
  server,
  initialMessage,
  onSave,
  onForget,
  onDone,
}: {
  server: SavedServer;
  initialMessage?: { tone: Tone; text: string };
  onSave: (s: SavedServer) => void;
  onForget: () => void;
  onDone: () => void;
}) {
  const [name, setName] = useState(server.name);
  const [list, setList] = useState(server.addresses);
  const [tests, setTests] = useState<Record<string, TestState>>({});
  const [newRoute, setNewRoute] = useState("");
  const [msg, setMsg] = useState<{ tone: Tone; text: string; addAnyway?: string } | null>(initialMessage ?? null);
  const [adding, setAdding] = useState(false);

  const save = (patch: Partial<SavedServer>) => {
    const next = { ...server, name, addresses: list, ...patch };
    onSave(next);
  };
  const setAddresses = (a: string[]) => {
    setList(a);
    save({ addresses: a });
  };

  const testOne = async (a: string) => {
    const id = await desktop.check(a).catch(() => null);
    setTests((t) => ({ ...t, [a]: id === server.id ? "ok" : id ? "other" : "down" }));
  };

  const testAdd = async () => {
    const v = newRoute.trim();
    if (!v) return;
    setAdding(true);
    try {
      const p = await desktop.probe(v);
      const a = "address" in p ? p.address : withPort(v);
      if (list.some((x) => sameAddress(x, a))) {
        setMsg({ tone: "warn", text: `⚠ ${a} is already in the list.` });
      } else if (p.kind === "foreman" && p.server_id === server.id) {
        setAddresses([...list, a]);
        setTests((t) => ({ ...t, [a]: "ok" }));
        setNewRoute("");
        setMsg({ tone: "ok", text: `✓ ${a} answered with the same server ID. Added as backup ${list.length}.` });
      } else if (p.kind === "foreman") {
        setMsg({
          tone: "bad",
          text: `✖ ${a} is a different Foreman server (${p.name}). Not added. Save it as its own server from the start screen instead.`,
        });
      } else {
        setMsg({
          tone: "warn",
          text: `⚠ ${describe(p, []).title.replace(/^. /, "")}. Not added. Fix the address, or add it anyway only if the server is off right now.`,
          addAnyway: p.kind === "no_answer" ? a : undefined,
        });
      }
    } finally {
      setAdding(false);
    }
  };

  return (
    <AuthShell label="Not connected" crumb="Saved server">
      <div className="auth__form">
        <div>
          <h1 className="auth__title">Edit server</h1>
          <p className="auth__lead">
            All addresses below reach the same server. The app tries them top to bottom and uses the first that
            answers.
          </p>
        </div>
        <label className="auth__field">
          <span>Name</span>
          <input
            type="text"
            className="auth__input"
            value={name}
            onChange={(e) => setName(e.target.value)}
            onBlur={() => save({ name: name.trim() || server.name })}
          />
          <span className="auth__hint auth__mono">Server ID {server.id} · checked on every address</span>
        </label>
        <section aria-label="Addresses" className="connect__panel">
          <div className="connect__panel-head auth__cap">Addresses · in order</div>
          {list.map((a, i) => {
            const st = tests[a] ?? "none";
            const [label, tone] = TEST_LABEL[st];
            return (
              <div key={a} className="connect__list-row">
                <span className="auth__mono connect__role">{i === 0 ? "Primary" : `Backup ${i}`}</span>
                <div className="connect__grow">
                  <span className="auth__mono" style={{ fontSize: 13 }}>{a}</span>
                  <div className="auth__hint">{addressKind(a)}</div>
                </div>
                <span className={`connect__tag connect--${tone}`}>{label}</span>
                <button type="button" className="auth__btn auth__btn--sm connect__icon-btn" onClick={() => void testOne(a)}>
                  Test
                </button>
                <button
                  type="button"
                  className="auth__btn auth__btn--sm connect__icon-btn"
                  aria-label="Move up"
                  disabled={i === 0}
                  onClick={() => {
                    const n = [...list];
                    [n[i - 1], n[i]] = [n[i], n[i - 1]];
                    setAddresses(n);
                  }}
                >
                  ↑
                </button>
                <button
                  type="button"
                  className="auth__btn auth__btn--sm"
                  disabled={list.length === 1}
                  onClick={() => setAddresses(list.filter((_, j) => j !== i))}
                >
                  Remove
                </button>
              </div>
            );
          })}
          <form
            className="connect__add"
            onSubmit={(e) => {
              e.preventDefault();
              void testAdd();
            }}
          >
            <input
              type="text"
              className="auth__input auth__mono connect__add-input"
              value={newRoute}
              onChange={(e) => setNewRoute(e.target.value)}
              placeholder="Add address · e.g. 100.101.5.7 or my-pi.local"
              aria-label="Add address"
            />
            <button type="submit" className="auth__btn auth__btn--sm" disabled={adding}>
              {adding ? "Testing…" : "Test and add"}
            </button>
          </form>
          {msg && (
            <div className="connect__pad">
              <Notice tone={msg.tone} title={msg.text} />
              {msg.addAnyway && (
                <button
                  type="button"
                  className="auth__btn auth__btn--sm"
                  style={{ marginTop: "var(--fw-space-2)" }}
                  onClick={() => {
                    const a = msg.addAnyway as string;
                    setAddresses([...list, a]);
                    setNewRoute("");
                    setMsg({ tone: "warn", text: `⚠ Added ${a} untested. It is only used if it answers with this server's ID.` });
                  }}
                >
                  Add anyway
                </button>
              )}
            </div>
          )}
        </section>
        <div className="auth__actions">
          <button
            type="button"
            className="auth__btn auth__btn--pri"
            onClick={() => {
              save({ name: name.trim() || server.name });
              onDone();
            }}
          >
            Done
          </button>
          <button type="button" className="auth__btn connect__danger" onClick={onForget}>
            Forget this server
          </button>
        </div>
      </div>
    </AuthShell>
  );
}

// ---------------------------------------------------------------------------
// Lost connection (every address down)
// ---------------------------------------------------------------------------

const ADDR_STATE: Record<AddrState, [string, Tone | "muted"]> = {
  ok: ["✓ Connected", "ok"],
  down: ["✖ No answer", "bad"],
  trying: ["… Trying", "warn"],
  standby: ["○ Standby", "muted"],
  unknown: ["○ Not checked", "muted"],
};

export function LostView({
  serverName,
  addresses,
  states,
  onRetry,
  onSwitch,
}: {
  serverName: string;
  addresses: string[];
  states: AddrState[];
  onRetry: () => void;
  onSwitch: () => void;
}) {
  return (
    <div className="connect__overlay">
      <AuthShell label={`${addresses[0] ?? ""} · reconnecting`} crumb="Connection">
        <div className="auth__form">
          <Notice
            tone="warn"
            title={`${serverName} stopped answering`}
            body="No address answers right now. Trying them in order with the same sign-in. Printers and the queue keep running on the server."
          />
          <section aria-label="Routes" className="connect__panel">
            <div className="connect__panel-head auth__cap">Routes to {serverName}</div>
            {addresses.map((a, i) => {
              const [label, tone] = ADDR_STATE[states[i] ?? "unknown"];
              return (
                <div key={a} className="connect__list-row">
                  <span className={`connect__tag connect__state connect--${tone}`}>{label}</span>
                  <div className="connect__grow">
                    <span className="auth__mono" style={{ fontSize: 13 }}>{a}</span>
                    <div className="auth__hint">
                      {i === 0 ? "Primary" : `Backup ${i}`} · {addressKind(a)}
                    </div>
                  </div>
                </div>
              );
            })}
          </section>
          <div className="auth__actions">
            <button type="button" className="auth__btn auth__btn--pri" onClick={onRetry}>
              Retry now
            </button>
            <button type="button" className="auth__btn" onClick={onSwitch}>
              Switch server
            </button>
          </div>
        </div>
      </AuthShell>
    </div>
  );
}
