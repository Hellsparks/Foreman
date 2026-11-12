/**
 * Settings → This app (desktop only: background + start with the computer) and
 * Settings → About (versions, data folders, updates, licence and source).
 */
import { useEffect, useState } from "react";
import { useLocation } from "react-router-dom";
import { api } from "../api/client";
import type { HealthResponse } from "../api/types/HealthResponse";
import { IS_TAURI } from "../shell/TopBar";
import Button from "../ui/Button";
import ExternalLink from "../ui/ExternalLink";
import { desktop, type AppInfo, type DesktopPrefs } from "./bridge";
import { useRoute } from "./route";
import { checkForUpdate, installUpdate, useUpdate } from "./updates";
import { versionMismatch } from "./versionCompat";
import "./Desktop.css";
import "../auth/Auth.css";

const REPO = "https://github.com/Hellsparks/Foreman";

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export function AppSettings() {
  const [prefs, setPrefs] = useState<DesktopPrefs | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    void desktop.prefs().then(setPrefs).catch((e) => setError(errText(e)));
  }, []);

  const apply = (p: Promise<DesktopPrefs>) =>
    void p
      .then((next) => {
        setPrefs(next);
        setError("");
      })
      .catch((e) => setError(errText(e)));

  return (
    <section aria-labelledby="app-heading">
      <h2 id="app-heading" className="dk-h2">
        This app
      </h2>
      {!prefs ? (
        <p className="auth__hint">{error || "Loading…"}</p>
      ) : (
        <div className="dk-stack">
          <label className="auth__check">
            <input
              type="checkbox"
              checked={prefs.keep_running}
              disabled={!prefs.tray_available}
              onChange={(e) => apply(desktop.setKeepRunning(e.target.checked))}
            />
            <span>
              Keep running in the background
              <span className="auth__hint">
                {prefs.tray_available
                  ? " Closing the window leaves Foreman in the tray, so the built-in server keeps your fleet reachable. Quit from the tray icon to stop it."
                  : " No tray is available on this desktop, so closing the window quits."}
                {prefs.keep_running_choice === null &&
                  (prefs.local_in_use ? " On by default while this computer is the server." : " Off by default when connected to another server.")}
              </span>
            </span>
          </label>
          <label className="auth__check">
            <input type="checkbox" checked={prefs.autostart} onChange={(e) => apply(desktop.setAutostart(e.target.checked))} />
            <span>
              Start with the computer
              <span className="auth__hint">
                {" "}
                Opens Foreman when you log in. It only keeps the fleet reachable with “Keep running in the background” on —
                then it starts hidden in the tray.
              </span>
            </span>
          </label>
          {error && (
            <div role="alert" className="auth__alert">
              {error}
            </div>
          )}
        </div>
      )}
    </section>
  );
}

function Updates({ app }: { app: AppInfo }) {
  const u = useUpdate();
  const info = u.status === "available" || u.status === "installing" ? u.info : u.status === "error" ? u.info : undefined;
  return (
    <div className="dk-stack">
      <h3 className="auth__cap">Updates</h3>
      {info ? (
        <>
          <p className="auth__hint" style={{ margin: 0 }}>
            Foreman {info.version} is available (you have {app.version}){info.date ? `, released ${info.date.slice(0, 10)}` : ""}.
            Read what changes before installing. Installing stops the built-in server, then restarts the app.
          </p>
          <pre className="dk-notes" aria-label={`Release notes for ${info.version}`}>
            {info.notes?.trim() || "No release notes."}
          </pre>
          <div className="dk-actions">
            <Button variant="primary" disabled={u.status === "installing"} onClick={() => void installUpdate()}>
              {u.status === "installing" ? "Installing…" : `Install ${info.version} and restart`}
            </Button>
            <ExternalLink href={`${REPO}/releases/tag/v${info.version}`}>Release page</ExternalLink>
          </div>
        </>
      ) : (
        <div className="dk-actions">
          <Button disabled={u.status === "checking"} onClick={() => void checkForUpdate()}>
            {u.status === "checking" ? "Checking…" : "Check now"}
          </Button>
          <span className="auth__hint" role="status">
            {u.status === "current" && `You have the newest version (${app.version}).`}
          </span>
        </div>
      )}
      {u.status === "error" && (
        <span className="auth__hint auth__hint--warn" role="status">
          Couldn't {u.info ? "install the update" : "check for updates"}: {u.error}
        </span>
      )}
    </div>
  );
}

export function AboutSettings() {
  const route = useRoute();
  const { hash } = useLocation();
  const [health, setHealth] = useState<HealthResponse | null>(null);
  const [app, setApp] = useState<AppInfo | null>(null);

  useEffect(() => {
    void api.health().then(setHealth).catch(() => {});
    if (IS_TAURI) void desktop.appInfo().then(setApp).catch(() => {});
  }, []);

  useEffect(() => {
    if (hash === "#about-heading") document.getElementById("about-heading")?.scrollIntoView({ block: "start" });
  }, [hash, app]);

  const commit = health?.commit ?? app?.commit ?? "";
  const srcHref = commit && commit !== "unknown" ? `${REPO}/tree/${commit}` : REPO;
  const mismatch = app && health ? versionMismatch(app.version, health.version) : null;

  return (
    <section aria-labelledby="about-heading">
      <h2 id="about-heading" className="dk-h2">
        About
      </h2>
      <div className="dk-stack">
        <dl className="dk-facts">
          {app && (
            <>
              <dt>App version</dt>
              <dd className="auth__mono">{app.version}</dd>
            </>
          )}
          <dt>Server version</dt>
          <dd className="auth__mono">
            {health?.version ?? "…"}
            {route && ` · ${route.mode === "local" ? "this computer" : route.serverName}`}
          </dd>
          <dt>Server commit</dt>
          <dd className="auth__mono">{health ? `${health.commit} · built ${health.built_at}` : "…"}</dd>
          {app && app.commit !== health?.commit && (
            <>
              <dt>App commit</dt>
              <dd className="auth__mono">{app.commit}</dd>
            </>
          )}
          {app && (
            <>
              <dt>App data</dt>
              <dd className="auth__mono">{app.app_data_dir}</dd>
            </>
          )}
          {app && route?.mode === "local" && (
            <>
              <dt>Server data</dt>
              <dd className="auth__mono">{app.server_data_dir}</dd>
            </>
          )}
        </dl>
        {mismatch && (
          <div className="dk-notice dk-notice--warn" role="note">
            <span className="dk-notice__text">{mismatch.message}</span>
          </div>
        )}
        {app && <Updates app={app} />}
        <h3 className="auth__cap">Licence</h3>
        <p className="auth__hint" style={{ margin: 0 }}>
          Foreman is free software under the GNU General Public License, version 3 or later. You may copy, change and
          share it under those terms; it comes with no warranty. The source for the version you are running is at{" "}
          <ExternalLink href={srcHref}>{commit ? `${REPO.replace("https://", "")} @ ${commit}` : undefined}</ExternalLink>.
        </p>
        <div className="dk-actions">
          <ExternalLink href={`${REPO}/blob/${commit || "main"}/LICENSE`}>Licence text (GPL-3.0)</ExternalLink>
          <ExternalLink href={REPO}>Source code</ExternalLink>
        </div>
      </div>
    </section>
  );
}
