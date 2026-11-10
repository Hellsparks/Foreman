import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { api } from "../api/client";
import { baseFor, getConnection, setConnection } from "../api/conn";
import AuthGate from "../auth/AuthGate";
import { AuthShell } from "../auth/AuthScreens";
import "../auth/Auth.css";
import { addressKind, sameAddress } from "./addresses";
import { desktop, type DesktopStore, type LocalStarted, type SavedServer } from "./bridge";
import { ConnectView, EditView, LocalView, LostView, WelcomeView } from "./ConnectViews";
import { Failover, type FailoverState } from "./failover";
import { RouteContext } from "./route";

type Msg = { tone: "ok" | "warn" | "bad"; text: string };

type Screen =
  | { kind: "loading" }
  | { kind: "welcome" }
  | { kind: "local" }
  | { kind: "connect" }
  | { kind: "edit"; id: string; msg?: Msg }
  | { kind: "remote"; id: string; first?: string }
  | { kind: "local-session"; lan: boolean };

/** Desktop app root: server picker, saved servers, Mode A, and per-server sessions. */
export default function DesktopRoot({ children }: { children: ReactNode }) {
  const [store, setStore] = useState<DesktopStore>({});
  const storeRef = useRef<DesktopStore>({});
  const [screen, setScreen] = useState<Screen>({ kind: "loading" });
  const [localErr, setLocalErr] = useState("");

  const update = useCallback((fn: (s: DesktopStore) => DesktopStore) => {
    const next = fn(storeRef.current);
    storeRef.current = next;
    setStore(next);
    void desktop.saveStore(next).catch((e) => console.error("saving servers failed", e));
  }, []);

  useEffect(() => {
    void desktop
      .loadStore()
      .catch(() => ({}) as DesktopStore)
      .then((s) => {
        storeRef.current = s;
        setStore(s);
        if (s.last === "local") setScreen({ kind: "local-session", lan: false });
        else if (s.last && s.servers?.some((x) => x.id === s.last)) setScreen({ kind: "remote", id: s.last });
        else setScreen({ kind: "welcome" });
      });
  }, []);

  const saved = store.servers ?? [];
  const upsert = (srv: SavedServer) =>
    update((s) => {
      const list = s.servers ?? [];
      const i = list.findIndex((x) => x.id === srv.id);
      const servers = i < 0 ? [...list, srv] : list.map((x, j) => (j === i ? srv : x));
      return { ...s, servers };
    });

  const open = (id: string, first?: string) => {
    update((s) => ({
      ...s,
      last: id,
      servers: (s.servers ?? []).map((x) => (x.id === id ? { ...x, last_used: Date.now() } : x)),
    }));
    setScreen({ kind: "remote", id, first });
  };

  const switchServer = useCallback(() => {
    update((s) => ({ ...s, last: null }));
    setScreen({ kind: "welcome" });
  }, [update]);

  switch (screen.kind) {
    case "loading":
      return <AuthShell label="Not connected" crumb="Starting" />;
    case "welcome":
      return (
        <WelcomeView
          saved={saved}
          onOpen={(id) => open(id)}
          onEdit={(id) => setScreen({ kind: "edit", id })}
          onRemote={() => setScreen({ kind: "connect" })}
          onLocal={() => {
            setLocalErr("");
            setScreen({ kind: "local" });
          }}
        />
      );
    case "local":
      return (
        <LocalView
          error={localErr}
          onBack={() => setScreen({ kind: "welcome" })}
          onStart={async (lan) => {
            try {
              await desktop.startLocal();
              update((s) => ({ ...s, last: "local" }));
              setScreen({ kind: "local-session", lan });
            } catch (e) {
              setLocalErr(String(e));
            }
          }}
        />
      );
    case "connect":
      return (
        <ConnectView
          saved={saved}
          onBack={() => setScreen({ kind: "welcome" })}
          onContinue={(srv) => {
            upsert(srv);
            open(srv.id);
          }}
          onAddBackup={(id, address) => {
            const srv = saved.find((x) => x.id === id);
            if (!srv) return;
            if (!srv.addresses.some((a) => sameAddress(a, address))) {
              upsert({ ...srv, addresses: [...srv.addresses, address] });
            }
            setScreen({ kind: "edit", id, msg: { tone: "ok", text: `✓ Added ${address} as a backup route.` } });
          }}
          onOpenSaved={(id, address) => open(id, address)}
        />
      );
    case "edit": {
      const srv = saved.find((x) => x.id === screen.id);
      if (!srv) return null;
      return (
        <EditView
          key={srv.id}
          server={srv}
          initialMessage={screen.msg}
          onSave={upsert}
          onForget={() => {
            update((s) => ({
              ...s,
              last: s.last === srv.id ? null : s.last,
              servers: (s.servers ?? []).filter((x) => x.id !== srv.id),
            }));
            setScreen({ kind: "welcome" });
          }}
          onDone={() => setScreen({ kind: "welcome" })}
        />
      );
    }
    case "remote": {
      const srv = saved.find((x) => x.id === screen.id);
      if (!srv) return null;
      return (
        <RemoteSession
          key={`${srv.id}|${screen.first ?? ""}`}
          server={srv}
          first={screen.first}
          onToken={(token) => upsert({ ...srv, token })}
          onSwitch={switchServer}
        >
          {children}
        </RemoteSession>
      );
    }
    case "local-session":
      return (
        <LocalSession
          lan={screen.lan}
          storedToken={store.local?.token ?? null}
          onToken={(token) => update((s) => ({ ...s, local: { ...s.local, token } }))}
          onSwitch={switchServer}
        >
          {children}
        </LocalSession>
      );
  }
}

// ---------------------------------------------------------------------------
// Mode B: one saved server, failover across its addresses
// ---------------------------------------------------------------------------

function RemoteSession({
  server,
  first,
  onToken,
  onSwitch,
  children,
}: {
  server: SavedServer;
  first?: string;
  onToken: (token: string | null) => void;
  onSwitch: () => void;
  children: ReactNode;
}) {
  const addresses = useMemo(
    () => (first ? [first, ...server.addresses.filter((a) => !sameAddress(a, first))] : server.addresses),
    // Only rebuild when the address list really changes.
    [first, server.addresses.join("|")],
  );
  const tokenRef = useRef(server.token);
  const onTokenRef = useRef(onToken);
  onTokenRef.current = onToken;
  const [fs, setFs] = useState<FailoverState | null>(null);
  const [current, setCurrent] = useState<string | null>(null);
  const managerRef = useRef<Failover | null>(null);

  useEffect(() => {
    const m = new Failover(server.id, addresses, desktop.check, (addr) => {
      if (managerRef.current !== m) return;
      setConnection(baseFor(addr), tokenRef.current);
      setCurrent(addr);
    });
    managerRef.current = m;
    const off = m.subscribe(setFs);
    setFs(m.snapshot);
    void m.start();
    return () => {
      off();
      m.stop();
      managerRef.current = null;
      // After the app's own cleanups (event stream) have run.
      queueMicrotask(() => setConnection("", null));
    };
  }, [server.id, addresses]);

  const retry = () => void managerRef.current?.retryNow();
  const lost = fs?.status === "lost";

  if (!current) {
    if (lost && fs) {
      return (
        <LostView serverName={server.name} addresses={addresses} states={fs.perAddress} onRetry={retry} onSwitch={onSwitch} />
      );
    }
    return <AuthShell label={addresses[0] ?? ""} crumb={`Connecting to ${server.name}`} />;
  }

  const active = fs && fs.active >= 0 ? fs.active : 0;
  return (
    <RouteContext.Provider
      value={{
        mode: "remote",
        serverName: server.name,
        address: current,
        via: active > 0 ? addressKind(current) : null,
        switchServer: onSwitch,
      }}
    >
      <AuthGate
        desktop={{
          address: current,
          onToken: (t) => {
            tokenRef.current = t;
            setConnection(getConnection().base, t);
            onTokenRef.current(t);
          },
          onSignedOut: () => {
            tokenRef.current = null;
            setConnection(getConnection().base, null);
            onTokenRef.current(null);
          },
          onChangeServer: onSwitch,
        }}
      >
        {children}
      </AuthGate>
      {lost && fs && (
        <LostView serverName={server.name} addresses={addresses} states={fs.perAddress} onRetry={retry} onSwitch={onSwitch} />
      )}
    </RouteContext.Provider>
  );
}

// ---------------------------------------------------------------------------
// Mode A: the embedded server
// ---------------------------------------------------------------------------

/** Turns on "Let other devices connect" once signed in (needs the admin from setup). */
function EnableLan() {
  useEffect(() => {
    void api.setNetwork(true).catch((e) => console.error("enable LAN failed", e));
  }, []);
  return null;
}

function LocalSession({
  lan,
  storedToken,
  onToken,
  onSwitch,
  children,
}: {
  lan: boolean;
  storedToken: string | null;
  onToken: (token: string) => void;
  onSwitch: () => void;
  children: ReactNode;
}) {
  const [started, setStarted] = useState<LocalStarted | null>(null);
  const [error, setError] = useState("");
  const onTokenRef = useRef(onToken);
  onTokenRef.current = onToken;
  const storedRef = useRef(storedToken);

  useEffect(() => {
    let live = true;
    void (async () => {
      try {
        const s = await desktop.startLocal();
        const token = storedRef.current ?? (await desktop.localToken());
        if (!live) return;
        setConnection(baseFor(s.address), token);
        onTokenRef.current(token);
        setStarted(s);
      } catch (e) {
        if (live) setError(String(e));
      }
    })();
    return () => {
      live = false;
      queueMicrotask(() => setConnection("", null));
    };
  }, []);

  const remint = async () => {
    const t = await desktop.localToken();
    setConnection(getConnection().base, t);
    onTokenRef.current(t);
  };

  if (!started) {
    return (
      <AuthShell label="This computer" crumb="Starting the built-in server">
        {error && (
          <div className="auth__form">
            <div role="alert" className="auth__alert">{error}</div>
            <div className="auth__actions">
              <button type="button" className="auth__btn" onClick={onSwitch}>
                Back
              </button>
            </div>
          </div>
        )}
      </AuthShell>
    );
  }

  return (
    <RouteContext.Provider
      value={{ mode: "local", serverName: "This computer", address: started.address, via: null, switchServer: onSwitch }}
    >
      <AuthGate
        desktop={{
          address: started.address,
          onToken: (t) => {
            setConnection(getConnection().base, t);
            onTokenRef.current(t);
          },
          onSignedOut: remint,
          onChangeServer: onSwitch,
          setupCode: started.setup_code ?? undefined,
          forceSetup: lan,
        }}
      >
        {lan && <EnableLan />}
        {children}
      </AuthGate>
    </RouteContext.Provider>
  );
}
