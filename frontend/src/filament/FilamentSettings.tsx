/**
 * Settings → Filament (doc 07 §A): tracking mode (Full · External · Off) and the
 * separate switch for serving the Spoolman API. A mode change shows the server's
 * preview of what will happen and needs a confirm.
 */
import { useCallback, useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { trackingApi, type FilamentMode, type FilamentSettings as Settings, type SwitchPreview } from "../api/tracking";
import Button from "../ui/Button";
import { useFilament } from "./policy";

const MODES: { mode: FilamentMode; title: string; means: string }[] = [
  { mode: "full", title: "Full", means: "Built-in Spoolman: inventory, load checks, reservations and deduction." },
  {
    mode: "external",
    title: "External",
    means: "Bring records over once from another Spoolman (as the setup wizard does), then track here as in Full.",
  },
  { mode: "off", title: "Off", means: "A fleet dashboard: Send, queue, projects and history without any filament inventory." },
];

const h2 = { font: "500 18px/1.3 var(--fw-sans)", margin: "0 0 var(--fw-space-4)", letterSpacing: "-0.02em" } as const;
const box = {
  border: "var(--fw-border-width) solid var(--fw-border)",
  background: "var(--fw-raised)",
  padding: "var(--fw-space-3) var(--fw-space-4)",
  display: "flex",
  gap: "var(--fw-space-3)",
  alignItems: "flex-start",
  cursor: "pointer",
} as const;

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export default function FilamentSettings() {
  const { reload } = useFilament();
  const [s, setS] = useState<Settings | null>(null);
  const [preview, setPreview] = useState<SwitchPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [note, setNote] = useState("");

  const load = useCallback(async () => {
    try {
      setS(await trackingApi.get());
    } catch (e) {
      setError(errText(e));
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load]);

  const act = async (f: () => Promise<void>) => {
    setBusy(true);
    setError("");
    try {
      await f();
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const pick = (mode: FilamentMode) =>
    act(async () => {
      setNote("");
      setPreview(mode === s?.policy.mode ? null : await trackingApi.preview(mode));
    });

  const confirm = (p: SwitchPreview) =>
    act(async () => {
      const r = await trackingApi.setMode(p.to);
      setS(r.settings);
      setPreview(null);
      const parts = [`Filament tracking is now ${MODES.find((m) => m.mode === p.to)?.title ?? p.to}.`];
      if (r.released_reservations) parts.push(`${r.released_reservations} reservations released.`);
      if (r.cleared_project_slots) parts.push(`${r.cleared_project_slots} project slots cleared.`);
      setNote(parts.join(" "));
      await reload();
    });

  const serve = (on: boolean) =>
    act(async () => {
      setS((await trackingApi.setServeApi(on)).settings);
    });

  if (!s) {
    return (
      <section aria-labelledby="filament-heading">
        <h2 id="filament-heading" style={h2}>Filament</h2>
        <p className="auth__hint">{error || "Loading…"}</p>
      </section>
    );
  }

  const current = s.policy.mode;
  const shown = preview?.to ?? current;

  return (
    <section aria-labelledby="filament-heading">
      <h2 id="filament-heading" style={h2}>Filament</h2>
      <div role="radiogroup" aria-label="Filament tracking" style={{ display: "flex", flexDirection: "column", gap: 2 }}>
        {MODES.map((m) => (
          <label key={m.mode} style={{ ...box, boxShadow: shown === m.mode ? "inset 3px 0 0 var(--fw-accent-hover)" : undefined }}>
            <input
              type="radio"
              name="filament-mode"
              checked={shown === m.mode}
              disabled={busy}
              onChange={() => void pick(m.mode)}
              style={{ marginTop: 3 }}
            />
            <span style={{ display: "flex", flexDirection: "column", gap: 2 }}>
              <b>
                {m.title}
                {m.mode === current && <span style={{ color: "var(--fw-muted)", fontWeight: "normal" }}> · current</span>}
              </b>
              <span style={{ fontSize: 13, color: "var(--fw-muted)" }}>{m.means}</span>
            </span>
          </label>
        ))}
      </div>

      {preview && (
        <div
          role="alertdialog"
          aria-labelledby="filament-confirm-h"
          style={{
            marginTop: "var(--fw-space-3)",
            border: "var(--fw-border-width) solid var(--fw-warning)",
            padding: "var(--fw-space-3) var(--fw-space-4)",
            background: "var(--fw-raised)",
          }}
        >
          <h3 id="filament-confirm-h" style={{ margin: "0 0 var(--fw-space-2)", fontSize: 14 }}>Confirm</h3>
          {preview.confirm.map((p, i) => (
            <p key={i} style={{ margin: "0 0 var(--fw-space-2)", fontSize: 13 }}>{p}</p>
          ))}
          {preview.to === "external" && (
            <p style={{ margin: "0 0 var(--fw-space-2)", fontSize: 13 }}>
              The import itself runs in <Link to="/setup">Setup → Filament data</Link>.
            </p>
          )}
          <div style={{ display: "flex", gap: "var(--fw-space-2)" }}>
            <Button variant="primary" disabled={busy} onClick={() => void confirm(preview)}>
              {preview.to === "off" ? "Turn tracking off" : "Switch"}
            </Button>
            <Button variant="secondary" disabled={busy} onClick={() => setPreview(null)}>
              Cancel
            </Button>
          </div>
        </div>
      )}

      <label style={{ ...box, marginTop: "var(--fw-space-4)" }}>
        <input type="checkbox" checked={s.serve_api} disabled={busy} onChange={(e) => void serve(e.target.checked)} style={{ marginTop: 3 }} />
        <span style={{ display: "flex", flexDirection: "column", gap: 2 }}>
          <b>Serve the Spoolman API{s.api_port != null ? ` on port ${s.api_port}` : ""}</b>
          <span style={{ fontSize: 13, color: "var(--fw-muted)" }}>
            For Moonraker and slicers that point at it. Separate from tracking, so turning tracking off never cuts them
            off silently. {s.api_running ? "Listening now." : "Not listening."}
          </span>
        </span>
      </label>

      {note && <p className="auth__hint auth__hint--ok" role="status">✓ {note}</p>}
      {error && <p className="auth__hint auth__hint--warn" role="alert">⚠ {error}</p>}
    </section>
  );
}
