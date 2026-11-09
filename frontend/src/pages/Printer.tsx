import { useState, useEffect, useCallback, useRef } from "react";
import { useParams, useNavigate, useOutletContext } from "react-router-dom";
import { api } from "../api/client";
import type { PrinterWithStatus, PrinterCapabilities, MacroFavourite } from "../api/client";
import type { PrinterStatus } from "../api/types/PrinterStatus";
import type { ShellContext } from "../shell/Shell";
import { BackButton, StatusChip, SectionLabel, Button } from "../ui";
import type { Tone } from "../ui/StatusChip";
import "./Printer.css";

// ---------------------------------------------------------------------------
// Status helpers (shared with Fleet)
// ---------------------------------------------------------------------------

function statusTone(state: string): Tone {
  switch (state) {
    case "printing": return "info";
    case "paused":   return "warn";
    case "error":    return "bad";
    case "offline":  return "off";
    default:         return "ok";
  }
}
function statusSymbol(state: string): string {
  switch (state) {
    case "printing": return "▶";
    case "paused":   return "‖";
    case "error":    return "✖";
    case "offline":  return "○";
    default:         return "●";
  }
}
function statusWord(state: string): string {
  switch (state) {
    case "printing": return "Printing";
    case "paused":   return "Paused";
    case "error":    return "Error";
    case "offline":  return "Offline";
    default:         return "Idle";
  }
}

// ---------------------------------------------------------------------------
// Constants from the design spec
// ---------------------------------------------------------------------------

const STEPS = [0.1, 1, 10, 25, 50, 100];
const BABY_STEPS = [-0.05, -0.01, 0.01, 0.05];
const EXT_LENGTHS = [10, 25, 50, 100];
const EXT_SPEEDS = [2, 5, 10];
const TABS = ["Live", "Queue", "Spools", "Service", "History", "Files", "Config"] as const;
const DEFAULT_PRESETS = [
  { label: "PLA",  nozzle: 215, bed: 60 },
  { label: "PETG", nozzle: 245, bed: 85 },
  { label: "ABS",  nozzle: 255, bed: 100 },
  { label: "Off",  nozzle: 0,   bed: 0 },
];

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------

export default function Printer() {
  const { id } = useParams<{ id: string }>();
  const printerId = Number(id);
  const navigate = useNavigate();
  const { statuses } = useOutletContext<ShellContext>();

  // Printer metadata
  const [printer, setPrinter] = useState<PrinterWithStatus | null>(null);
  const [caps, setCaps] = useState<PrinterCapabilities | null>(null);
  const [favourites, setFavourites] = useState<MacroFavourite[]>([]);

  // Tab
  const [activeTab, setActiveTab] = useState<typeof TABS[number]>("Live");

  // Live status: prefer streaming from shell context, fall back to fetched
  const liveStatus: PrinterStatus | null = statuses[printerId] ?? printer?.status ?? null;
  const state = liveStatus?.state ?? "offline";
  const isPrinting = state === "printing" || state === "paused";

  // Toolhead state
  const [step, setStep] = useState(10);
  const [extLen, setExtLen] = useState(10);
  const [extSpd, setExtSpd] = useState(5);

  // Console
  const [consoleLines, setConsoleLines] = useState<string[]>([]);
  const [gcodeInput, setGcodeInput] = useState("");
  const [gcodeHistory, setGcodeHistory] = useState<string[]>([]);
  const [historyIdx, setHistoryIdx] = useState(-1);
  const consoleRef = useRef<HTMLDivElement>(null);

  // Webcam
  const [activeCam, setActiveCam] = useState(0);
  const [camTs, setCamTs] = useState(Date.now());

  // Cancel dialog
  const [cancelOpen, setCancelOpen] = useState(false);
  const [cancelling, setCancelling] = useState(false);

  // All macros vs favourites
  const [showAllMacros, setShowAllMacros] = useState(false);

  // ---------------------------------------------------------------------------
  // Load printer data on mount
  // ---------------------------------------------------------------------------
  useEffect(() => {
    if (!printerId) return;
    api.listPrinters().then((list) => {
      const p = list.find((x) => x.id === printerId);
      if (p) setPrinter(p);
    }).catch(() => {});
    api.getCapabilities(printerId).then(setCaps).catch(() => {});
    api.listFavourites(printerId).then(setFavourites).catch(() => {});
  }, [printerId]);

  // Refresh snapshot every 10s
  useEffect(() => {
    const timer = setInterval(() => setCamTs(Date.now()), 10_000);
    return () => clearInterval(timer);
  }, []);

  // ---------------------------------------------------------------------------
  // Command helpers
  // ---------------------------------------------------------------------------
  const cmd = useCallback(
    async (kind: string, body: Record<string, unknown> = {}) => {
      try {
        await api.sendCommand(printerId, kind, body);
        addConsoleLine(`> ${kind} ${JSON.stringify(body)}`);
      } catch (e: unknown) {
        const msg = e instanceof Error ? e.message : String(e);
        addConsoleLine(`! ${kind}: ${msg}`);
      }
    },
    [printerId],
  );

  function addConsoleLine(line: string) {
    setConsoleLines((prev) => [...prev.slice(-200), line]);
    requestAnimationFrame(() => {
      consoleRef.current?.scrollTo(0, consoleRef.current.scrollHeight);
    });
  }

  const sendGcode = useCallback(() => {
    const trimmed = gcodeInput.trim();
    if (!trimmed) return;
    setGcodeHistory((h) => [...h, trimmed]);
    setHistoryIdx(-1);
    setGcodeInput("");
    cmd("gcode", { command: trimmed });
  }, [gcodeInput, cmd]);

  const handleCancel = useCallback(async () => {
    setCancelling(true);
    try {
      const { confirm_token } = await api.confirmCancel(printerId);
      await api.sendCommand(printerId, "cancel", { confirm_token });
      addConsoleLine("> cancel (confirmed)");
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      addConsoleLine(`! cancel: ${msg}`);
    } finally {
      setCancelling(false);
      setCancelOpen(false);
    }
  }, [printerId]);

  // ---------------------------------------------------------------------------
  // Derived data
  // ---------------------------------------------------------------------------
  const nozzleActual = liveStatus?.nozzle?.actual ?? 0;
  const nozzleTarget = liveStatus?.nozzle?.target ?? 0;
  const bedActual = liveStatus?.bed?.actual ?? 0;
  const bedTarget = liveStatus?.bed?.target ?? 0;
  const nozzleCold = nozzleActual < 170;
  const extLocked = isPrinting || nozzleCold;

  const webcams = caps?.webcams ?? [];
  const macros = caps?.macros ?? [];
  const favNames = new Set(favourites.map((f) => f.macro_name));
  const displayMacros = showAllMacros
    ? macros
    : macros.filter((m) => favNames.has(m));

  const fwTag = caps?.firmware_version ?? printer?.firmware ?? "";

  if (!printer) {
    return <div className="printer-placeholder">Loading...</div>;
  }

  // ---------------------------------------------------------------------------
  // Render
  // ---------------------------------------------------------------------------
  return (
    <>
      {/* ── Header ─────────────────────────────────────────────── */}
      <header className="printer-header">
        <BackButton label="Fleet" onClick={() => navigate("/fleet")} />
        <div className="printer-header__top">
          <h1 className="printer-header__name">{printer.name}</h1>
          {fwTag && <span className="printer-header__fw">{fwTag}</span>}
          <StatusChip
            tone={statusTone(state)}
            symbol={statusSymbol(state)}
            word={statusWord(state)}
          />
          <div className="printer-header__spacer" />
          <div className="printer-header__actions">
            {isPrinting && (
              <>
                <Button
                  variant="secondary"
                  onClick={() =>
                    cmd(state === "paused" ? "resume" : "pause")
                  }
                >
                  {state === "paused" ? "Resume" : "Pause"}
                </Button>
                <Button
                  variant="secondary"
                  onClick={() => setCancelOpen(true)}
                  style={{ borderColor: "var(--fw-danger)", color: "var(--fw-danger)" }}
                >
                  Cancel print...
                </Button>
              </>
            )}
            {printer.primary_url && (
              <a
                href={printer.primary_url}
                target="_blank"
                rel="noopener noreferrer"
                className="ui-btn ui-btn--secondary"
                style={{ display: "inline-flex", alignItems: "center", gap: 6, textDecoration: "none" }}
              >
                Native UI
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="square" aria-hidden="true">
                  <path d="M14 4h6v6M20 4l-9 9M18 14v6H4V6h6" />
                </svg>
              </a>
            )}
          </div>
        </div>

        {/* Tab bar */}
        <div className="printer-tabs" role="tablist" aria-label="Printer sections">
          {TABS.map((t) => (
            <button
              key={t}
              role="tab"
              aria-selected={t === activeTab}
              className={`printer-tab${t === activeTab ? " printer-tab--active" : ""}`}
              onClick={() => setActiveTab(t)}
            >
              {t}
            </button>
          ))}
        </div>
      </header>

      {/* ── Tab content ────────────────────────────────────────── */}
      {activeTab === "Live" ? (
        <div className="printer-live" role="tabpanel" aria-label="Live">

          {/* LEFT COLUMN: webcams, console, macros */}
          <div className="printer-live__left">

            {/* Webcam section */}
            <section aria-label="Webcams" style={{ border: "1px solid var(--fw-border)", background: "var(--fw-raised)" }}>
              <div className="webcam-bar">
                <div role="tablist" aria-label="Choose webcam" style={{ display: "flex", flexWrap: "wrap" }}>
                  {webcams.map((wc, i) => (
                    <button
                      key={wc.name}
                      role="tab"
                      aria-selected={i === activeCam}
                      className={`webcam-tab${i === activeCam ? " webcam-tab--active" : ""}`}
                      onClick={() => setActiveCam(i)}
                    >
                      {wc.name}
                    </button>
                  ))}
                  {webcams.length === 0 && (
                    <span style={{ padding: "0 14px", lineHeight: "44px", color: "var(--fw-muted)", fontSize: 13 }}>
                      No webcams
                    </span>
                  )}
                </div>
              </div>
              <div className="webcam-view">
                {webcams.length > 0 ? (
                  <img
                    src={`${api.snapshotUrl(printerId)}?t=${camTs}&cam=${activeCam}`}
                    alt={`${webcams[activeCam]?.name ?? "Webcam"} live view`}
                  />
                ) : (
                  <span style={{ color: "var(--fw-muted)" }}>No camera configured</span>
                )}
              </div>
            </section>

            {/* Console */}
            <section className="live-panel live-panel--gap-sm" aria-label="Console" style={{ padding: "12px 16px" }}>
              <SectionLabel as="h2">Console</SectionLabel>
              <div className="console-output" ref={consoleRef}>
                {consoleLines.length === 0
                  ? "Ready — type a G-code command below."
                  : consoleLines.join("\n")}
              </div>
              <div className="console-input">
                <label style={{ position: "absolute", width: 1, height: 1, overflow: "hidden" }}>G-code command</label>
                <input
                  type="text"
                  placeholder="Send G-code, ↑ for history"
                  value={gcodeInput}
                  onChange={(e) => setGcodeInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      sendGcode();
                    } else if (e.key === "ArrowUp") {
                      e.preventDefault();
                      if (gcodeHistory.length > 0) {
                        const idx = historyIdx < 0 ? gcodeHistory.length - 1 : Math.max(0, historyIdx - 1);
                        setHistoryIdx(idx);
                        setGcodeInput(gcodeHistory[idx]);
                      }
                    } else if (e.key === "ArrowDown") {
                      e.preventDefault();
                      if (historyIdx >= 0) {
                        const idx = historyIdx + 1;
                        if (idx >= gcodeHistory.length) {
                          setHistoryIdx(-1);
                          setGcodeInput("");
                        } else {
                          setHistoryIdx(idx);
                          setGcodeInput(gcodeHistory[idx]);
                        }
                      }
                    }
                  }}
                />
                <button type="button" onClick={sendGcode}>Send</button>
              </div>
            </section>

            {/* Macros */}
            <section className="live-panel live-panel--gap" aria-label="Macros" style={{ padding: "12px 16px" }}>
              <div className="macros-header">
                <SectionLabel as="h2" style={{ flex: 1 }}>
                  Macros {showAllMacros ? "" : "· favourites"}
                </SectionLabel>
                <Button
                  variant="small"
                  onClick={() => setShowAllMacros((v) => !v)}
                >
                  {showAllMacros ? "Favourites" : `All ${macros.length}`}
                </Button>
              </div>
              <div className="macro-grid">
                {displayMacros.map((m) => (
                  <button
                    key={m}
                    className="macro-btn"
                    onClick={() => cmd("macro", { name: m })}
                  >
                    {m}
                  </button>
                ))}
                {displayMacros.length === 0 && (
                  <span style={{ color: "var(--fw-muted)", fontSize: 13 }}>
                    {showAllMacros ? "No macros discovered" : "No favourites set"}
                  </span>
                )}
              </div>
            </section>
          </div>

          {/* RIGHT COLUMN: temps, toolhead, extruder */}
          <div className="printer-live__right">

            {/* Temperatures */}
            <section className="live-panel" aria-label="Temperatures">
              <div className="preset-bar">
                <SectionLabel as="h2" style={{ flex: 1 }}>Temps</SectionLabel>
                {DEFAULT_PRESETS.map((pr) => (
                  <button
                    key={pr.label}
                    className="preset-btn"
                    onClick={() => {
                      cmd("set_nozzle_temp", { target: pr.nozzle });
                      cmd("set_bed_temp", { target: pr.bed });
                    }}
                    aria-label={`${pr.label} preset for all heaters`}
                  >
                    {pr.label}
                  </button>
                ))}
              </div>

              {/* Nozzle */}
              <div className="heater-row">
                <span className="heater-row__name">Nozzle</span>
                <span
                  className="heater-row__now"
                  style={{ color: nozzleActual > 50 ? "var(--fw-danger)" : undefined }}
                >
                  {nozzleActual.toFixed(0)}°
                </span>
                <span className="heater-row__arrow" aria-hidden="true">→</span>
                <input
                  type="number"
                  className="heater-row__input"
                  min={0}
                  max={300}
                  step={1}
                  value={nozzleTarget}
                  onChange={(e) => cmd("set_nozzle_temp", { target: Number(e.target.value) })}
                  aria-label="Nozzle target, max 300 °C"
                />
                <button
                  className="heater-row__off"
                  onClick={() => cmd("set_nozzle_temp", { target: 0 })}
                  aria-label="Turn nozzle off"
                >
                  Off
                </button>
              </div>

              {/* Bed */}
              <div className="heater-row">
                <span className="heater-row__name">Bed</span>
                <span
                  className="heater-row__now"
                  style={{ color: bedActual > 50 ? "var(--fw-danger)" : undefined }}
                >
                  {bedActual.toFixed(0)}°
                </span>
                <span className="heater-row__arrow" aria-hidden="true">→</span>
                <input
                  type="number"
                  className="heater-row__input"
                  min={0}
                  max={120}
                  step={1}
                  value={bedTarget}
                  onChange={(e) => cmd("set_bed_temp", { target: Number(e.target.value) })}
                  aria-label="Bed target, max 120 °C"
                />
                <button
                  className="heater-row__off"
                  onClick={() => cmd("set_bed_temp", { target: 0 })}
                  aria-label="Turn bed off"
                >
                  Off
                </button>
              </div>
            </section>

            {/* Toolhead */}
            <section className="live-panel live-panel--gap" aria-label="Toolhead">
              <SectionLabel as="h2">Toolhead</SectionLabel>
              {isPrinting && (
                <p style={{ margin: 0, fontSize: 12, color: "var(--fw-warning)" }}>
                  Movement locked while printing (babystep allowed)
                </p>
              )}

              {/* Jog pad + Z column + Home column */}
              <div className="jog-layout">
                {/* XY pad */}
                <div className="jog-pad" role="group" aria-label="Move X and Y">
                  {/* Row 1 */}
                  <span />
                  <button className="jog-btn" disabled={isPrinting} onClick={() => cmd("jog", { y: step })} aria-label="Move Y+">Y+</button>
                  <span />
                  {/* Row 2 */}
                  <button className="jog-btn" disabled={isPrinting} onClick={() => cmd("jog", { x: -step })} aria-label="Move X-">X−</button>
                  <button className="jog-btn jog-btn--home" disabled={isPrinting} onClick={() => cmd("home", { axes: ["X", "Y"] })} aria-label="Home XY">⌂</button>
                  <button className="jog-btn" disabled={isPrinting} onClick={() => cmd("jog", { x: step })} aria-label="Move X+">X+</button>
                  {/* Row 3 */}
                  <span />
                  <button className="jog-btn" disabled={isPrinting} onClick={() => cmd("jog", { y: -step })} aria-label="Move Y-">Y−</button>
                  <span />
                </div>

                {/* Z column */}
                <div className="jog-z-col" role="group" aria-label="Move Z">
                  <button className="jog-btn" disabled={isPrinting} onClick={() => cmd("jog", { z: step })} aria-label="Move Z up">Z+</button>
                  <button className="jog-btn jog-btn--home" disabled={isPrinting} onClick={() => cmd("home", { axes: ["Z"] })} aria-label="Home Z">⌂Z</button>
                  <button className="jog-btn" disabled={isPrinting} onClick={() => cmd("jog", { z: -step })} aria-label="Move Z down">Z−</button>
                </div>

                {/* Home all + Home X/Y + level */}
                <div className="jog-home-col" role="group" aria-label="Homing">
                  <button className="jog-btn jog-btn--primary" disabled={isPrinting} onClick={() => cmd("home")} style={{ width: "100%", height: 44 }}>
                    ⌂ Home all
                  </button>
                  <div className="home-xy-row">
                    <button className="jog-btn" disabled={isPrinting} onClick={() => cmd("home", { axes: ["X"] })} style={{ width: "100%", height: 44 }}>⌂ X</button>
                    <button className="jog-btn" disabled={isPrinting} onClick={() => cmd("home", { axes: ["Y"] })} style={{ width: "100%", height: 44 }}>⌂ Y</button>
                  </div>
                  <button className="jog-btn" disabled={isPrinting} onClick={() => cmd("motors_off")} style={{ width: "100%", height: 44, fontSize: 11 }}>
                    Motors off
                  </button>
                </div>
              </div>

              {/* Step selector */}
              <div className="step-bar" role="group" aria-label="Move distance in millimetres">
                {STEPS.map((s) => (
                  <button
                    key={s}
                    className={`step-btn${s === step ? " step-btn--active" : ""}`}
                    onClick={() => setStep(s)}
                    aria-pressed={s === step}
                  >
                    {s}
                  </button>
                ))}
              </div>

              {/* Babystep row */}
              <div className="babystep-row" role="group" aria-label="Z offset">
                <span className="babystep-row__label">ΔZ</span>
                {BABY_STEPS.map((b) => (
                  <button
                    key={b}
                    className="babystep-btn"
                    onClick={() => cmd("baby_step", { offset: b })}
                  >
                    {b > 0 ? `+${b}` : String(b)}
                  </button>
                ))}
              </div>
            </section>

            {/* Extruder */}
            <section className="live-panel live-panel--gap-sm" aria-label="Extruder">
              <SectionLabel as="h2">Extruder</SectionLabel>

              {/* Length selector */}
              <div className="ext-selector" role="group" aria-label="Extrude length in millimetres">
                <span className="ext-selector__label">mm</span>
                {EXT_LENGTHS.map((l) => (
                  <button
                    key={l}
                    className={`ext-btn${l === extLen ? " ext-btn--active" : ""}`}
                    onClick={() => setExtLen(l)}
                    aria-pressed={l === extLen}
                  >
                    {l}
                  </button>
                ))}
              </div>

              {/* Speed selector */}
              <div className="ext-selector" role="group" aria-label="Extrude speed in mm/s">
                <span className="ext-selector__label">mm/s</span>
                {EXT_SPEEDS.map((s) => (
                  <button
                    key={s}
                    className={`ext-btn${s === extSpd ? " ext-btn--active" : ""}`}
                    onClick={() => setExtSpd(s)}
                    aria-pressed={s === extSpd}
                  >
                    {s}
                  </button>
                ))}
              </div>

              {nozzleCold && (
                <p style={{ margin: 0, fontSize: 12, color: "var(--fw-warning)" }}>
                  Nozzle below 170 °C — extrusion locked
                </p>
              )}
              {isPrinting && !nozzleCold && (
                <p style={{ margin: 0, fontSize: 12, color: "var(--fw-warning)" }}>
                  Extrusion locked while printing
                </p>
              )}

              <div className="ext-actions">
                <button
                  disabled={extLocked}
                  onClick={() => cmd("extrude", { length: -extLen, speed: extSpd * 60 })}
                >
                  ↑ Retract
                </button>
                <button
                  disabled={extLocked}
                  onClick={() => cmd("extrude", { length: extLen, speed: extSpd * 60 })}
                >
                  ↓ Extrude
                </button>
              </div>
            </section>
          </div>
        </div>
      ) : (
        <div className="printer-placeholder" role="tabpanel" aria-label={activeTab}>
          <p>{activeTab} — coming soon</p>
          <p style={{ fontSize: 13 }}>This tab will be implemented in a future phase.</p>
        </div>
      )}

      {/* Cancel confirmation dialog */}
      {cancelOpen && (
        <div className="cancel-dialog-overlay" onClick={() => setCancelOpen(false)}>
          <div className="cancel-dialog" onClick={(e) => e.stopPropagation()}>
            <h2>Cancel print?</h2>
            <p>This will stop the current print. The printer will cool down.</p>
            <div className="cancel-dialog__actions">
              <Button variant="secondary" onClick={() => setCancelOpen(false)}>
                Keep printing
              </Button>
              <Button
                variant="primary"
                onClick={handleCancel}
                disabled={cancelling}
                style={{ background: "var(--fw-danger)", borderColor: "var(--fw-danger)" }}
              >
                {cancelling ? "Cancelling..." : "Cancel print"}
              </Button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
