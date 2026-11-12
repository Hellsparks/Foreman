import { useState, useEffect } from "react";
import { useAppearance, THEMES, MODES, type Theme, type Mode } from "../hooks/useAppearance";
import { api } from "../api/client";
import type { PrinterWithStatus, PrinterPath, PathTestResult } from "../api/client";
import SecuritySettings from "../auth/SecuritySettings";
import BackupSettings from "../backup/BackupSettings";
import ThemeSettings from "../theme/ThemeSettings";
import RerunSetup from "../setup/RerunSetup";
import { EdgePanel } from "../ui";

// Accent preview colours per theme × mode — matches tokens.css accent-hover values.
// Used only for the swatch dots; never hardcoded in component logic.
const ACCENT_PREVIEW: Record<Theme, Record<Mode, string>> = {
  fieldwork: { light: "#BF3D1C", dark: "#F57951" },
  vector:    { light: "#455900", dark: "#D5FA39" },
  foundry:   { light: "#3D3D3D", dark: "#D5D5D5" },
  olive:     { light: "#5D622D", dark: "#C2C78D" },
};

const THEME_LABELS: Record<Theme, string> = {
  fieldwork: "Fieldwork",
  vector:    "Vector",
  foundry:   "Foundry",
  olive:     "Olive",
};

export default function Settings() {
  const { theme, mode, setTheme, setMode } = useAppearance();
  const [printers, setPrinters] = useState<PrinterWithStatus[]>([]);
  const [expandedId, setExpandedId] = useState<number | null>(null);

  useEffect(() => {
    api.listPrinters().then(setPrinters).catch(() => {});
  }, []);

  const toggleExpand = (id: number) => {
    setExpandedId((prev) => (prev === id ? null : id));
  };

  return (
    <div className="set-page auto-cols">
      {/* Phone (doc 06 §2): the settings groups in the left panel. Desktop has no sidebar here. */}
      <EdgePanel side="left" label="Settings groups" storageKey="settings.groups" desktop={false}>
        <span className="edge-panel__label">Settings</span>
        <nav className="set-groups" aria-label="Settings groups">
          {[
            ["appearance-heading", "Appearance"],
            ["printers-heading", "Printers"],
            ["security-heading", "Security"],
            ["backup-heading", "Backup"],
            ["setup-heading", "Setup"],
          ].map(([id, label]) => (
            <a key={id} href={`#${id}`} className="set-groups__link" onClick={(e) => {
              e.preventDefault();
              document.getElementById(id)?.scrollIntoView({ block: "start" });
            }}>
              {label}
            </a>
          ))}
        </nav>
      </EdgePanel>
      <div className="page-header span-all">
        <p className="page-header__eyebrow">Settings</p>
        <h1 className="page-header__title phone-dup-title">Settings</h1>
      </div>

      {/* Appearance ─────────────────────────────────────────────────────── */}
      <section aria-labelledby="appearance-heading">
        <h2
          id="appearance-heading"
          style={{ font: "500 18px/1.3 var(--fw-sans)", margin: "0 0 var(--fw-space-4)", letterSpacing: "-0.02em" }}
        >
          Appearance
        </h2>

        {/* Theme selector */}
        <fieldset style={{ border: 0, padding: 0, margin: "0 0 var(--fw-space-6)" }}>
          <legend
            style={{
              fontFamily: "var(--fw-mono)",
              fontSize: 12,
              letterSpacing: "0.06em",
              textTransform: "uppercase",
              color: "var(--fw-muted)",
              marginBottom: 10,
              padding: 0,
            }}
          >
            Theme
          </legend>
          <div className="appearance-grid">
            {THEMES.map((t) => (
              <button
                key={t}
                type="button"
                className="appearance-swatch"
                aria-label={`${THEME_LABELS[t]} theme`}
                aria-pressed={theme === t}
                onClick={() => setTheme(t)}
              >
                <span
                  className="appearance-swatch__dot"
                  style={{ background: ACCENT_PREVIEW[t][mode] }}
                />
                {THEME_LABELS[t]}
              </button>
            ))}
          </div>
        </fieldset>

        {/* Mode selector */}
        <fieldset style={{ border: 0, padding: 0, margin: 0 }}>
          <legend
            style={{
              fontFamily: "var(--fw-mono)",
              fontSize: 12,
              letterSpacing: "0.06em",
              textTransform: "uppercase",
              color: "var(--fw-muted)",
              marginBottom: 10,
              padding: 0,
            }}
          >
            Mode
          </legend>
          <div style={{ display: "flex", gap: "var(--fw-space-2)" }}>
            {MODES.map((m) => (
              <button
                key={m}
                type="button"
                className="appearance-swatch"
                style={{ flex: "1 1 auto" }}
                aria-label={`${m} mode`}
                aria-pressed={mode === m}
                onClick={() => setMode(m)}
              >
                {m === "light" ? (
                  <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden="true">
                    <circle cx="12" cy="12" r="4" />
                    <path d="M12 2v2M12 20v2M2 12h2M20 12h2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" />
                  </svg>
                ) : (
                  <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden="true">
                    <path d="M21 12.79A9 9 0 1111.21 3 7 7 0 0021 12.79z" />
                  </svg>
                )}
                {m.charAt(0).toUpperCase() + m.slice(1)}
              </button>
            ))}
          </div>
        </fieldset>

        <ThemeSettings />
      </section>

      {/* Printers ───────────────────────────────────────────────────────── */}
      <section aria-labelledby="printers-heading">
        <h2
          id="printers-heading"
          style={{ font: "500 18px/1.3 var(--fw-sans)", margin: "0 0 var(--fw-space-4)", letterSpacing: "-0.02em" }}
        >
          Printers
        </h2>
        {printers.length === 0 ? (
          <p style={{ color: "var(--fw-muted)", fontSize: 13 }}>No printers registered.</p>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            {printers.map((p) => (
              <div key={p.id} style={{ border: "var(--fw-border-width) solid var(--fw-border)", background: "var(--fw-raised)" }}>
                <div
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: "var(--fw-space-3)",
                    padding: "var(--fw-space-3) var(--fw-space-4)",
                  }}
                >
                  <span style={{ flex: 1, fontWeight: 500 }}>{p.name}</span>
                  <span
                    style={{
                      fontFamily: "var(--fw-mono)",
                      fontSize: 11,
                      color: "var(--fw-muted)",
                      textTransform: "uppercase",
                    }}
                  >
                    {p.firmware}
                  </span>
                  {p.active_path_index != null && p.active_path_index > 0 && p.active_path_name && (
                    <span style={{ fontSize: 11, color: "var(--fw-warning)", fontFamily: "var(--fw-mono)" }}>
                      via {p.active_path_name}
                    </span>
                  )}
                  <button
                    type="button"
                    onClick={() => toggleExpand(p.id)}
                    aria-expanded={expandedId === p.id}
                    style={{
                      minHeight: 32,
                      padding: "0 10px",
                      border: "var(--fw-border-width) solid var(--fw-border)",
                      background: "transparent",
                      borderRadius: "var(--fw-radius)",
                      cursor: "pointer",
                      fontSize: 12,
                    }}
                  >
                    {expandedId === p.id ? "Collapse" : "Configure"}
                  </button>
                </div>
                {expandedId === p.id && (
                  <div style={{ borderTop: "var(--fw-border-width) solid var(--fw-border)", padding: "var(--fw-space-4)" }}>
                    <SettingsPrinterConfigForm printer={p} />
                    <RemovePrinter
                      printer={p}
                      onRemoved={() => {
                        setPrinters((prev) => prev.filter((x) => x.id !== p.id));
                        setExpandedId(null);
                      }}
                    />
                  </div>
                )}
              </div>
            ))}
          </div>
        )}
      </section>

      <SecuritySettings />

      <BackupSettings />

      <RerunSetup />

      <p className="page-placeholder span-all">
        Server config, backup, notifications — coming in phase 7.
      </p>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Remove printer (type the name to confirm)
// ---------------------------------------------------------------------------

function RemovePrinter({ printer, onRemoved }: { printer: PrinterWithStatus; onRemoved: () => void }) {
  const [open, setOpen] = useState(false);
  const [typed, setTyped] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");

  const remove = async () => {
    setBusy(true);
    setErr("");
    try {
      await api.deletePrinter(printer.id);
      onRemoved();
    } catch (e: unknown) {
      setErr(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  };

  const btn = {
    minHeight: "var(--fw-control-h-sm)",
    padding: "0 14px",
    border: "var(--fw-border-width) solid var(--fw-danger)",
    color: "var(--fw-danger)",
    background: "transparent",
    borderRadius: "var(--fw-radius)",
    cursor: "pointer",
    fontSize: 13,
  } as const;

  return (
    <div
      style={{
        marginTop: "var(--fw-space-4)",
        paddingTop: "var(--fw-space-4)",
        borderTop: "var(--fw-border-width) solid var(--fw-border)",
        display: "flex",
        flexWrap: "wrap",
        alignItems: "center",
        gap: 10,
      }}
    >
      <div style={{ flex: "1 1 240px" }}>
        <div style={{ fontWeight: "bold", fontSize: 13 }}>Remove printer</div>
        <div style={{ fontSize: 12, color: "var(--fw-muted)" }}>
          Removes it from Foreman (paths, SSH settings, macros). Job history is kept.
        </div>
      </div>
      {!open ? (
        <button type="button" style={btn} onClick={() => setOpen(true)}>
          Remove printer…
        </button>
      ) : (
        <>
          <input
            type="text"
            autoFocus
            aria-label={`Type ${printer.name} to confirm`}
            placeholder={`Type “${printer.name}” to confirm`}
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
            style={{
              minHeight: "var(--fw-control-h-sm)",
              border: "var(--fw-border-width) solid var(--fw-danger)",
              background: "var(--fw-raised)",
              padding: "0 10px",
              borderRadius: "var(--fw-radius)",
              fontSize: 13,
            }}
          />
          <button
            type="button"
            style={{ ...btn, background: "var(--fw-danger)", color: "var(--fw-on-accent)" }}
            disabled={typed !== printer.name || busy}
            onClick={() => void remove()}
          >
            {busy ? "Removing…" : "Remove"}
          </button>
          <button
            type="button"
            style={{ ...btn, borderColor: "var(--fw-muted)", color: "var(--fw-text)" }}
            onClick={() => {
              setOpen(false);
              setTyped("");
            }}
          >
            Cancel
          </button>
        </>
      )}
      {err && <span style={{ flexBasis: "100%", fontSize: 13, color: "var(--fw-danger)" }}>{err}</span>}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Settings inline printer config form
// ---------------------------------------------------------------------------

interface LocalPathS {
  id?: number;
  name: string;
  url: string;
  priority: number;
  healthy: boolean | null;
  latency_ms: number | null;
  deleted?: boolean;
}

function SettingsPrinterConfigForm({ printer }: { printer: PrinterWithStatus }) {
  const [paths, setPaths] = useState<LocalPathS[]>([]);
  const [failAfter, setFailAfter] = useState(printer.fail_after_checks ?? 3);
  const [returnAfter, setReturnAfter] = useState(printer.return_after_secs ?? 60);
  const [reporter, setReporter] = useState(printer.usage_reporter ?? "auto");
  const [authValue, setAuthValue] = useState(
    printer.firmware === "bambu" ? (printer.access_code ?? "") : (printer.api_key ?? "")
  );
  const [testMsg, setTestMsg] = useState("");
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);
  const [savedMsg, setSavedMsg] = useState("");

  useEffect(() => {
    api.listPaths(printer.id).then((list) => {
      setPaths(
        list.map((p) => ({
          id: p.id,
          name: p.name,
          url: p.url,
          priority: p.priority,
          healthy: null,
          latency_ms: null,
        }))
      );
    }).catch(() => {});
  }, [printer.id]);

  const activePaths = paths.filter((p) => !p.deleted);

  const addPath = () => {
    setPaths((prev) => [
      ...prev,
      {
        name: `Path ${prev.filter((p) => !p.deleted).length + 1}`,
        url: "",
        priority: prev.filter((p) => !p.deleted).length * 10,
        healthy: null,
        latency_ms: null,
      },
    ]);
  };

  const removePath = (idx: number) => {
    setPaths((prev) =>
      prev.map((p, i) => (i === idx ? { ...p, deleted: true } : p))
    );
  };

  const testAll = async () => {
    setTesting(true);
    setTestMsg("Testing…");
    try {
      const results: PathTestResult[] = await api.testAllPaths(printer.id);
      const resultMap = new Map(results.map((r) => [r.id, r]));
      setPaths((prev) =>
        prev.map((p) => {
          if (p.id == null) return p;
          const r = resultMap.get(p.id);
          if (!r) return p;
          return { ...p, healthy: r.reachable, latency_ms: r.latency_ms };
        })
      );
      const ok = results.filter((r) => r.reachable).length;
      setTestMsg(`${ok}/${results.length} paths reachable`);
    } catch {
      setTestMsg("Test failed");
    } finally {
      setTesting(false);
    }
  };

  const save = async () => {
    setSaving(true);
    setSavedMsg("");
    try {
      await api.updatePrinter(printer.id, {
        fail_after_checks: failAfter,
        return_after_secs: returnAfter,
        usage_reporter: reporter,
        ...(printer.firmware === "bambu"
          ? { access_code: authValue }
          : { api_key: authValue }),
      });

      for (const p of paths) {
        if (p.deleted && p.id != null) {
          await api.deletePath(printer.id, p.id);
        } else if (!p.deleted && p.id == null) {
          await api.createPath(printer.id, { name: p.name, url: p.url, priority: p.priority });
        } else if (!p.deleted && p.id != null) {
          await api.updatePath(printer.id, p.id, { name: p.name, url: p.url, priority: p.priority });
        }
      }

      const updated = await api.listPaths(printer.id);
      setPaths(updated.map((p: PrinterPath) => ({
        id: p.id,
        name: p.name,
        url: p.url,
        priority: p.priority,
        healthy: null,
        latency_ms: null,
      })));

      setSavedMsg("Saved");
      setTimeout(() => setSavedMsg(""), 3000);
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      setSavedMsg(`Error: ${msg}`);
    } finally {
      setSaving(false);
    }
  };

  const authLabel = printer.firmware === "bambu" ? "Access code" : "API key (optional)";

  const FAIL_OPTIONS = [
    { label: "1 failed check (≈2 s)", value: 1 },
    { label: "3 failed checks (≈6 s)", value: 3 },
    { label: "5 failed checks (≈10 s)", value: 5 },
  ];
  const RETURN_OPTIONS = [
    { label: "healthy for 10 s", value: 10 },
    { label: "healthy for 60 s", value: 60 },
    { label: "healthy for 10 min", value: 600 },
    { label: "never (manual only)", value: 999999 },
  ];

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--fw-space-4)" }}>
      {/* Paths */}
      <div style={{ display: "flex", flexDirection: "column", gap: "var(--fw-space-2)" }}>
        <span style={{ fontSize: 13, fontWeight: "bold" }}>
          Connection paths
          <span style={{ fontWeight: "normal", color: "var(--fw-muted)" }}> · tried top to bottom</span>
        </span>
        {activePaths.map((p, i) => {
          const pathIdx = paths.indexOf(p);
          return (
            <div
              key={pathIdx}
              style={{
                display: "grid",
                gridTemplateColumns: "26px minmax(90px, 0.8fr) minmax(0, 1.6fr) auto",
                alignItems: "center",
                gap: "var(--fw-space-2)",
                padding: "6px var(--fw-space-2)",
                border: "var(--fw-border-width) solid var(--fw-border)",
                background: "var(--fw-surface)",
              }}
            >
              <span
                style={{
                  fontFamily: "var(--fw-mono)",
                  fontSize: 12,
                  fontWeight: "bold",
                  color: "var(--fw-muted)",
                  textAlign: "center",
                }}
              >
                {i + 1}
              </span>
              <input
                type="text"
                value={p.name}
                onChange={(e) =>
                  setPaths((prev) =>
                    prev.map((x, xi) => xi === pathIdx ? { ...x, name: e.target.value } : x)
                  )
                }
                aria-label={`Path ${i + 1} name`}
                style={{
                  minHeight: 32,
                  border: "var(--fw-border-width) solid var(--fw-muted)",
                  background: "var(--fw-raised)",
                  padding: "0 var(--fw-space-2)",
                  borderRadius: "var(--fw-radius)",
                  fontSize: 13,
                  width: "100%",
                  boxSizing: "border-box",
                }}
              />
              <input
                type="text"
                value={p.url}
                onChange={(e) =>
                  setPaths((prev) =>
                    prev.map((x, xi) => xi === pathIdx ? { ...x, url: e.target.value } : x)
                  )
                }
                spellCheck={false}
                aria-label={`Path ${i + 1} URL`}
                style={{
                  minHeight: 32,
                  border: "var(--fw-border-width) solid var(--fw-muted)",
                  background: "var(--fw-raised)",
                  padding: "0 var(--fw-space-2)",
                  borderRadius: "var(--fw-radius)",
                  fontFamily: "var(--fw-mono)",
                  fontSize: 12,
                  width: "100%",
                  boxSizing: "border-box",
                }}
              />
              <button
                type="button"
                onClick={() => removePath(pathIdx)}
                disabled={activePaths.length <= 1}
                aria-label={`Remove path ${i + 1}`}
                style={{
                  width: 32,
                  height: 32,
                  border: "var(--fw-border-width) solid var(--fw-border)",
                  background: "transparent",
                  cursor: "pointer",
                  borderRadius: "var(--fw-radius)",
                }}
              >
                ✕
              </button>
            </div>
          );
        })}
        <div style={{ display: "flex", gap: "var(--fw-space-2)", flexWrap: "wrap", alignItems: "center" }}>
          <button
            type="button"
            onClick={addPath}
            style={{
              minHeight: 32,
              padding: "0 10px",
              border: "var(--fw-border-width) dashed var(--fw-muted)",
              background: "transparent",
              borderRadius: "var(--fw-radius)",
              fontSize: 13,
              cursor: "pointer",
            }}
          >
            + Add path
          </button>
          <button
            type="button"
            onClick={testAll}
            disabled={testing}
            style={{
              minHeight: 32,
              padding: "0 10px",
              border: "var(--fw-border-width) solid var(--fw-muted)",
              background: "transparent",
              borderRadius: "var(--fw-radius)",
              fontSize: 13,
              cursor: "pointer",
            }}
          >
            {testing ? "Testing…" : "Test all"}
          </button>
          {testMsg && (
            <span style={{ fontSize: 12, color: "var(--fw-muted)" }}>{testMsg}</span>
          )}
        </div>
      </div>

      {/* Failover settings */}
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(auto-fit, minmax(200px, 1fr))",
          gap: "var(--fw-space-2)",
        }}
      >
        <label style={{ display: "flex", flexDirection: "column", gap: "var(--fw-space-1)" }}>
          <span style={{ fontSize: 12, fontWeight: "bold" }}>Switch after</span>
          <select
            value={failAfter}
            onChange={(e) => setFailAfter(Number(e.target.value))}
            style={{
              minHeight: 32,
              border: "var(--fw-border-width) solid var(--fw-muted)",
              background: "var(--fw-raised)",
              padding: "0 var(--fw-space-2)",
              borderRadius: "var(--fw-radius)",
            }}
          >
            {FAIL_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>{o.label}</option>
            ))}
          </select>
        </label>
        <label style={{ display: "flex", flexDirection: "column", gap: "var(--fw-space-1)" }}>
          <span style={{ fontSize: 12, fontWeight: "bold" }}>Go back up when</span>
          <select
            value={returnAfter}
            onChange={(e) => setReturnAfter(Number(e.target.value))}
            style={{
              minHeight: 32,
              border: "var(--fw-border-width) solid var(--fw-muted)",
              background: "var(--fw-raised)",
              padding: "0 var(--fw-space-2)",
              borderRadius: "var(--fw-radius)",
            }}
          >
            {RETURN_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>{o.label}</option>
            ))}
          </select>
        </label>
        <label style={{ display: "flex", flexDirection: "column", gap: "var(--fw-space-1)" }}>
          <span style={{ fontSize: 12, fontWeight: "bold" }}>Filament use reported by</span>
          <select
            value={reporter}
            onChange={(e) => setReporter(e.target.value as "auto" | "moonraker" | "foreman")}
            style={{
              minHeight: 32,
              border: "var(--fw-border-width) solid var(--fw-muted)",
              background: "var(--fw-raised)",
              padding: "0 var(--fw-space-2)",
              borderRadius: "var(--fw-radius)",
            }}
          >
            <option value="auto">Auto (Moonraker if linked to Spoolman)</option>
            <option value="moonraker">Moonraker [spoolman]</option>
            <option value="foreman">Foreman (from the file)</option>
          </select>
        </label>
      </div>

      {/* Auth */}
      <label style={{ display: "flex", flexDirection: "column", gap: 6 }}>
        <span style={{ fontSize: 13, fontWeight: "bold" }}>{authLabel}</span>
        <input
          type="password"
          value={authValue}
          onChange={(e) => setAuthValue(e.target.value)}
          style={{
            minHeight: "var(--fw-control-h-sm)",
            border: "var(--fw-border-width) solid var(--fw-muted)",
            background: "var(--fw-raised)",
            padding: "0 10px",
            borderRadius: "var(--fw-radius)",
          }}
        />
      </label>

      {/* Save */}
      <div style={{ display: "flex", alignItems: "center", gap: "var(--fw-space-3)" }}>
        <button
          type="button"
          onClick={save}
          disabled={saving}
          style={{
            minHeight: "var(--fw-control-h-sm)",
            padding: "0 var(--fw-space-4)",
            border: "var(--fw-border-width) solid transparent",
            background: "var(--fw-accent-hover)",
            color: "var(--fw-on-accent)",
            borderRadius: "var(--fw-radius)",
            cursor: "pointer",
            fontFamily: "var(--fw-mono)",
            fontWeight: "bold",
          }}
        >
          {saving ? "Saving…" : "Save changes"}
        </button>
        {savedMsg && (
          <span
            role="status"
            style={{
              fontSize: 13,
              color: savedMsg.startsWith("Error") ? "var(--fw-danger)" : "var(--fw-success)",
            }}
          >
            {savedMsg}
          </span>
        )}
      </div>
    </div>
  );
}
