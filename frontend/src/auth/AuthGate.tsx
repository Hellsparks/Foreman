import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { api, ApiError, UNAUTHORIZED_EVENT, type ServerInfo } from "../api/client";
import { AuthShell, LoginView, SetupView } from "./AuthScreens";
import "./Auth.css";

interface AuthValue {
  username: string;
  logout: () => Promise<void>;
}

const AuthContext = createContext<AuthValue | null>(null);

/** Signed-in admin + logout. Only usable below `<AuthGate>`. */
export function useAuth(): AuthValue {
  const v = useContext(AuthContext);
  if (!v) throw new Error("useAuth outside AuthGate");
  return v;
}

type Phase =
  | { kind: "loading" }
  | { kind: "offline"; message: string }
  | { kind: "setup"; info: ServerInfo }
  | { kind: "login"; info: ServerInfo }
  | { kind: "ready"; info: ServerInfo; username: string };

/** Desktop-app hooks; the browser uses none of these (cookie session, same origin). */
export interface DesktopAuth {
  /** Shown in the frame and server row. */
  address: string;
  /** Store the bearer token from login/setup. */
  onToken: (token: string) => Promise<void> | void;
  /** The token stopped working (logout, expiry, revoked). Mode A mints a new one here. */
  onSignedOut: () => Promise<void> | void;
  onChangeServer?: () => void;
  /** Mode A: prefill the code the embedded server generated. */
  setupCode?: string;
  /** Mode A + "Let other devices connect": show setup even though the app token works. */
  forceSetup?: boolean;
}

/** Shows Setup or Sign-in until the server accepts our session. */
export default function AuthGate({ children, desktop }: { children: ReactNode; desktop?: DesktopAuth }) {
  const [phase, setPhase] = useState<Phase>({ kind: "loading" });
  const desktopRef = useRef(desktop);
  desktopRef.current = desktop;

  const resolve = useCallback(async () => {
    let info: ServerInfo;
    try {
      info = await api.serverInfo();
    } catch (e) {
      setPhase({ kind: "offline", message: e instanceof Error ? e.message : String(e) });
      return;
    }
    if (info.needs_setup && desktopRef.current?.forceSetup) {
      setPhase({ kind: "setup", info });
      return;
    }
    try {
      const me = await api.me();
      setPhase({ kind: "ready", info, username: me.username });
    } catch (e) {
      // 403 = unclaimed server (only health, info and setup answer).
      if (e instanceof ApiError && (e.status === 401 || (e.status === 403 && info.needs_setup))) {
        setPhase(info.needs_setup ? { kind: "setup", info } : { kind: "login", info });
      } else {
        setPhase({ kind: "offline", message: e instanceof Error ? e.message : String(e) });
      }
    }
  }, []);

  useEffect(() => {
    void resolve();
  }, [resolve]);

  // Any protected call that comes back 401 (session expired, revoked, server
  // reset) drops us back to the sign-in screen.
  useEffect(() => {
    const onUnauthorized = async () => {
      setPhase((p) => (p.kind === "ready" ? { kind: "loading" } : p));
      await desktopRef.current?.onSignedOut();
      await resolve();
    };
    const handler = () => void onUnauthorized();
    window.addEventListener(UNAUTHORIZED_EVENT, handler);
    return () => window.removeEventListener(UNAUTHORIZED_EVENT, handler);
  }, [resolve]);

  const logout = useCallback(async () => {
    await api.logout().catch(() => {});
    await desktopRef.current?.onSignedOut();
    await resolve();
  }, [resolve]);

  const signedIn = (info: ServerInfo) => async (username: string, token: string | undefined) => {
    if (token) await desktopRef.current?.onToken(token);
    setPhase({ kind: "ready", info, username });
  };

  const addr = desktop?.address ?? window.location.host;
  const common = {
    addr,
    returnToken: !!desktop,
    onChange: desktop?.onChangeServer,
  };

  switch (phase.kind) {
    case "loading":
      return <AuthShell label={addr} crumb="Connecting" />;
    case "offline":
      return (
        <AuthShell label="Not connected" crumb="Connection">
          <div className="auth__form">
            <h1 className="auth__title">Can't reach the server</h1>
            <div role="alert" className="auth__alert">{phase.message}</div>
            <div className="auth__actions">
              <button type="button" className="auth__btn auth__btn--pri" onClick={() => void resolve()}>
                Try again
              </button>
              {desktop?.onChangeServer && (
                <button type="button" className="auth__btn" onClick={desktop.onChangeServer}>
                  Switch server
                </button>
              )}
            </div>
          </div>
        </AuthShell>
      );
    case "setup":
      return (
        <SetupView
          info={phase.info}
          {...common}
          prefillCode={desktop?.setupCode}
          onDone={(u, t) => void signedIn(phase.info)(u, t)}
        />
      );
    case "login":
      return <LoginView info={phase.info} {...common} onDone={(u, t) => void signedIn(phase.info)(u, t)} />;
    case "ready":
      return (
        <AuthContext.Provider value={{ username: phase.username, logout }}>{children}</AuthContext.Provider>
      );
  }
}
