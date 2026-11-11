import { useState, useEffect, useCallback, useRef } from "react";
import type { CSSProperties, ReactNode } from "react";
import { useParams, useNavigate, useOutletContext } from "react-router-dom";
import { api } from "../api/client";
import type { PrinterWithStatus, PrinterCapabilities, MacroFavourite, PrinterPath, PathTestResult } from "../api/client";
import type { PrinterStatus } from "../api/types/PrinterStatus";
import type { ShellContext } from "../shell/Shell";
import { StatusChip, SectionLabel, Button } from "../ui";
import { createPortal } from "react-dom";
import { TOPBAR_SLOT_ID } from "../shell/TopBar";
import { FilesTab, SshSettings, TerminalTab } from "./PrinterFiles";
import PrinterSpoolsTab from "../filament/PrinterSpoolsTab";
import type { SshSettingsHandle } from "./PrinterFiles";
import type { Tone } from "../ui/StatusChip";
import { useSignedUrl } from "../hooks/useSignedUrl";
import { usePhone } from "../hooks/useMediaQuery";
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
const TABS = ["Live", "Queue", "Spools", "Service", "History", "Files", "Terminal", "Config"] as const;
const DEFAULT_PRESETS = [
  { label: "PLA",  nozzle: 215, bed: 60 },
  { label: "PETG", nozzle: 245, bed: 85 },
  { label: "ABS",  nozzle: 255, bed: 100 },
  { label: "Off",  nozzle: 0,   bed: 0 },
];

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------

function usePersistentFlag(key: string): [boolean, (f: (v: boolean) => boolean) => void] {
  const [value, setValue] = useState(() => localStorage.getItem(key) === "1");
  const update = useCallback(
    (f: (v: boolean) => boolean) =>
      setValue((v) => {
        const next = f(v);
        localStorage.setItem(key, next ? "1" : "0");
        return next;
      }),
    [key],
  );
  return [value, update];
}

const OVERLAY_ALPHA_KEY = "foreman.live.overlayOpacity";

function loadOverlayAlpha(): number {
  const v = Number(localStorage.getItem(OVERLAY_ALPHA_KEY));
  return Number.isFinite(v) && v >= 0.2 && v <= 1 ? v : 0.85;
}

/** A panel that slides in from the right edge over the webcam feed. */
function CamOverlay({
  label,
  onClose,
  width,
  actions,
  children,
}: {
  label: string;
  onClose: () => void;
  width: number;
  actions?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="cam-ov" aria-label={label} style={{ width }}>
      <div className="cam-ov__head">
        <SectionLabel as="h2" style={{ flex: 1 }}>{label}</SectionLabel>
        {actions}
        <button type="button" className="cam-ov__close" onClick={onClose} aria-label={`Hide ${label}`} title={`Hide ${label}`}>
          ›
        </button>
      </div>
      <div className="cam-ov__body">{children}</div>
    </section>
  );
}

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
  const [visited, setVisited] = useState<Set<string>>(() => new Set(["Live"]));
  useEffect(() => {
    setVisited((v) => (v.has(activeTab) ? v : new Set(v).add(activeTab)));
  }, [activeTab]);

  const phone = usePhone();
  const tabsRef = useRef<HTMLDivElement>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const menuBtnRef = useRef<HTMLButtonElement>(null);
  // The list is position:fixed (so it never widens the header); place it under the button.
  const menuPos = (): CSSProperties => {
    const r = menuBtnRef.current?.getBoundingClientRect();
    return r ? { top: r.bottom + 4, right: Math.max(8, window.innerWidth - r.right) } : {};
  };
  // Phone: the tab row scrolls sideways; keep the active tab in view.
  useEffect(() => {
    if (!phone) return;
    const el = tabsRef.current?.querySelector<HTMLElement>('[aria-selected="true"]');
    el?.scrollIntoView({ inline: "center", block: "nearest" });
  }, [phone, activeTab, printer]);

  const [topbarSlot, setTopbarSlot] = useState<HTMLElement | null>(null);
  useEffect(() => setTopbarSlot(document.getElementById(TOPBAR_SLOT_ID)), []);

  // Live status: prefer streaming from shell context, fall back to fetched
  const liveStatus: PrinterStatus | null = statuses[printerId] ?? printer?.status ?? null;
  const state = liveStatus?.state ?? "offline";
  const isPrinting = state === "printing" || state === "paused";

  // Temp input draft state (avoids WS overwriting while typing)
  const [nozzleDraft, setNozzleDraft] = useState("");
  const [bedDraft, setBedDraft] = useState("");
  const [nozzleFocused, setNozzleFocused] = useState(false);
  const [bedFocused, setBedFocused] = useState(false);

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
  const camSrc = useSignedUrl(printerId ? api.snapshotPath(printerId, camTs) : null);

  // Cancel dialog
  const [cancelOpen, setCancelOpen] = useState(false);
  const [cancelling, setCancelling] = useState(false);

  // All macros vs favourites
  const [showAllMacros, setShowAllMacros] = useState(false);

  // Live tab side columns (remembered per browser)
  const [consoleOpen, setConsoleOpen] = usePersistentFlag("foreman.live.consoleOpen");
  const [macrosOpen, setMacrosOpen] = usePersistentFlag("foreman.live.macrosOpen");
  const [overlayAlpha, setOverlayAlpha] = useState(loadOverlayAlpha);
  useEffect(() => localStorage.setItem(OVERLAY_ALPHA_KEY, String(overlayAlpha)), [overlayAlpha]);

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

  // Sync temp drafts from WS when the input is not focused
  useEffect(() => {
    if (!nozzleFocused) setNozzleDraft(String(nozzleTarget));
  }, [nozzleTarget, nozzleFocused]);

  useEffect(() => {
    if (!bedFocused) setBedDraft(String(bedTarget));
  }, [bedTarget, bedFocused]);

  /** Ensure a URL has a protocol prefix (bare IPs become http://). */
  const ensureProtocol = (url: string) =>
    /^https?:\/\//i.test(url) ? url : `http://${url}`;

  if (!printer) {
    return <div className="printer-placeholder">Loading...</div>;
  }

  // ---------------------------------------------------------------------------
  // Render
  // ---------------------------------------------------------------------------
  return (
    <>
      {/* ── Header ─────────────────────────────────────────────── */}
      {topbarSlot &&
        createPortal(
          <>
            <button
              type="button"
              className="printer-title__back"
              aria-label="Back to Fleet"
              title="Back to Fleet"
              onClick={() => navigate("/fleet")}
            >
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="square" aria-hidden="true">
                <path d="M15 5l-7 7 7 7" />
              </svg>
            </button>
            <h1 className="printer-title__name">{printer.name}</h1>
            {fwTag && <span className="printer-header__fw">{fwTag}</span>}
            {printer.active_path_index != null && printer.active_path_index > 0 && printer.active_path_name && (
              <span className="printer-header__via" style={{ color: "var(--fw-warning)", fontSize: 12, whiteSpace: "nowrap" }}>
                via {printer.active_path_name}
              </span>
            )}
            <StatusChip tone={statusTone(state)} symbol={statusSymbol(state)} word={statusWord(state)} />
          </>,
          topbarSlot,
        )}
      <header className="printer-header">
        <div className="printer-tabs" role="tablist" aria-label="Printer sections" ref={tabsRef}>
          {TABS.filter((t) => t !== "Terminal" || printer.firmware !== "bambu").map((t) => (
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
        <div className="printer-header__spacer" />
        {phone ? (
          (isPrinting || printer.primary_url) && (
            <div className="printer-menu">
              <button
                type="button"
                className="printer-menu__btn"
                ref={menuBtnRef}
                aria-label="More actions"
                aria-haspopup="menu"
                aria-expanded={menuOpen}
                onClick={() => setMenuOpen((o) => !o)}
                onKeyDown={(e) => e.key === "Escape" && setMenuOpen(false)}
              >
                <svg width="22" height="22" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
                  <circle cx="12" cy="5" r="2" /><circle cx="12" cy="12" r="2" /><circle cx="12" cy="19" r="2" />
                </svg>
              </button>
              {menuOpen && (
                <>
                  <div className="printer-menu__scrim" onClick={() => setMenuOpen(false)} />
                  <div className="printer-menu__list" role="menu" aria-label="Printer actions" style={menuPos()}>
                    {isPrinting && (
                      <>
                        <button
                          type="button"
                          role="menuitem"
                          className="printer-menu__item"
                          onClick={() => { setMenuOpen(false); cmd(state === "paused" ? "resume" : "pause"); }}
                        >
                          {state === "paused" ? "Resume" : "Pause"}
                        </button>
                        <button
                          type="button"
                          role="menuitem"
                          className="printer-menu__item printer-menu__item--danger"
                          onClick={() => { setMenuOpen(false); setCancelOpen(true); }}
                        >
                          Cancel print...
                        </button>
                      </>
                    )}
                    {printer.primary_url && (
                      <a
                        role="menuitem"
                        className="printer-menu__item"
                        href={ensureProtocol(printer.primary_url)}
                        target="_blank"
                        rel="noopener noreferrer"
                        onClick={() => setMenuOpen(false)}
                      >
                        Native UI
                        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="square" aria-hidden="true">
                          <path d="M14 4h6v6M20 4l-9 9M18 14v6H4V6h6" />
                        </svg>
                      </a>
                    )}
                  </div>
                </>
              )}
            </div>
          )
        ) : (
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
                href={ensureProtocol(printer.primary_url)}
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
        )}
      </header>

      {/* ── Tab content ────────────────────────────────────────── */}
      {activeTab === "Live" ? (
        <div className="printer-live" role="tabpanel" aria-label="Live">

          {/* Webcam: scales to fit the viewport */}
          <div className="printer-live__cam">
            <section aria-label="Webcams" style={{ border: "var(--fw-border-width) solid var(--fw-border)", background: "var(--fw-raised)" }}>
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
                    <span style={{ padding: "0 14px", lineHeight: "40px", color: "var(--fw-muted)", fontSize: 13 }}>
                      No webcams
                    </span>
                  )}
                </div>
                <div className="webcam-bar__spacer" />
                <div className="webcam-bar__tools">
                  <button
                    type="button"
                    className="webcam-bar__toggle"
                    aria-pressed={consoleOpen}
                    onClick={() => setConsoleOpen((v) => !v)}
                  >
                    Console
                  </button>
                  <button
                    type="button"
                    className="webcam-bar__toggle"
                    aria-pressed={macrosOpen}
                    onClick={() => setMacrosOpen((v) => !v)}
                  >
                    Macros
                  </button>
                </div>
              </div>
              <div className="webcam-view">
                {webcams.length > 0 ? (
                  <img
                    src={camSrc ?? undefined}
                    alt={`${webcams[activeCam]?.name ?? "Webcam"} live view`}
                  />
                ) : (
                  <span style={{ color: "var(--fw-muted)" }}>No camera configured</span>
                )}

                <div className="cam-ov-stack" style={{ "--cam-ov-alpha": `${Math.round(overlayAlpha * 100)}%` } as CSSProperties}>
          {consoleOpen && (
          <CamOverlay label="Console" onClose={() => setConsoleOpen(() => false)} width={340}>
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
          </CamOverlay>
          )}

          {macrosOpen && (
          <CamOverlay
            label={`Macros${showAllMacros ? "" : " · favourites"}`}
            onClose={() => setMacrosOpen(() => false)}
            width={280}
            actions={
              <Button variant="small" onClick={() => setShowAllMacros((v) => !v)}>
                {showAllMacros ? "Favourites" : `All ${macros.length}`}
              </Button>
            }
          >
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
          </CamOverlay>
          )}
                </div>
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
                  value={nozzleDraft}
                  onChange={(e) => setNozzleDraft(e.target.value)}
                  onFocus={() => setNozzleFocused(true)}
                  onBlur={() => {
                    setNozzleFocused(false);
                    const val = Number(nozzleDraft);
                    if (!isNaN(val) && val >= 0 && val <= 300 && val !== nozzleTarget) {
                      cmd("set_nozzle_temp", { target: val });
                    }
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      (e.target as HTMLInputElement).blur();
                    }
                  }}
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
                  value={bedDraft}
                  onChange={(e) => setBedDraft(e.target.value)}
                  onFocus={() => setBedFocused(true)}
                  onBlur={() => {
                    setBedFocused(false);
                    const val = Number(bedDraft);
                    if (!isNaN(val) && val >= 0 && val <= 120 && val !== bedTarget) {
                      cmd("set_bed_temp", { target: val });
                    }
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      (e.target as HTMLInputElement).blur();
                    }
                  }}
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
                  <button className="jog-btn jog-btn--primary" disabled={isPrinting} onClick={() => cmd("home")} style={{ width: "100%", height: "var(--fw-control-h)" }}>
                    ⌂ Home all
                  </button>
                  <div className="home-xy-row">
                    <button className="jog-btn" disabled={isPrinting} onClick={() => cmd("home", { axes: ["X"] })} style={{ width: "100%", height: "var(--fw-control-h)" }}>⌂ X</button>
                    <button className="jog-btn" disabled={isPrinting} onClick={() => cmd("home", { axes: ["Y"] })} style={{ width: "100%", height: "var(--fw-control-h)" }}>⌂ Y</button>
                  </div>
                  <button className="jog-btn" disabled={isPrinting} onClick={() => cmd("motors_off")} style={{ width: "100%", height: "var(--fw-control-h)", fontSize: 11 }}>
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
      ) : activeTab === "Files" || activeTab === "Terminal" ? null : activeTab === "Spools" ? (
        <PrinterSpoolsTab printerId={printerId} />
      ) : activeTab === "Config" ? (
        <PrinterConfigTab
          printerId={printerId}
          printer={printer}
          caps={caps}
          overlayAlpha={overlayAlpha}
          onOverlayAlpha={setOverlayAlpha}
        />
      ) : (
        <div className="printer-placeholder" role="tabpanel" aria-label={activeTab}>
          <p>{activeTab} — coming soon</p>
          <p style={{ fontSize: 13 }}>This tab will be implemented in a future phase.</p>
        </div>
      )}

      {/* Files and Terminal stay mounted once opened so SFTP state and the SSH shell survive tab switches. */}
      {visited.has("Files") && (
        <div className="printer-fill" hidden={activeTab !== "Files"}>
          <FilesTab printerId={printerId} firmware={printer.firmware} isPrinting={isPrinting} />
        </div>
      )}
      {visited.has("Terminal") && (
        <div className="printer-fill" hidden={activeTab !== "Terminal"}>
          <TerminalTab printerId={printerId} firmware={printer.firmware} />
        </div>
      )}

      {/* ── Config tab rendered outside the main layout ── */}

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

// ---------------------------------------------------------------------------
// PrinterConfigTab
// ---------------------------------------------------------------------------

interface LocalPath {
  /** undefined = new (not yet saved) */
  id?: number;
  name: string;
  url: string;
  priority: number;
  /** null = unknown, true/false = last test result */
  healthy: boolean | null;
  latency_ms: number | null;
  deleted?: boolean;
}

function PrinterConfigTab({
  printerId,
  printer,
  caps,
  overlayAlpha,
  onOverlayAlpha,
}: {
  printerId: number;
  printer: PrinterWithStatus;
  caps: PrinterCapabilities | null;
  overlayAlpha: number;
  onOverlayAlpha: (v: number) => void;
}) {
  const [paths, setPaths] = useState<LocalPath[]>([]);
  const [failAfter, setFailAfter] = useState(printer.fail_after_checks ?? 3);
  const [returnAfter, setReturnAfter] = useState(printer.return_after_secs ?? 60);
  const [hardenedNozzle, setHardenedNozzle] = useState(printer.hardened_nozzle);
  const [authValue, setAuthValue] = useState(
    printer.firmware === "bambu" ? (printer.access_code ?? "") : (printer.api_key ?? "")
  );
  const [testMsg, setTestMsg] = useState("");
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);
  const [savedMsg, setSavedMsg] = useState("");
  const [activePath, setActivePath] = useState<{ index: number; path: PrinterPath } | null>(null);
  const sshRef = useRef<SshSettingsHandle>(null);

  useEffect(() => {
    api.listPaths(printerId).then((list) => {
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
    api.getActivePath(printerId).then(setActivePath).catch(() => {});
  }, [printerId]);

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

  const movePath = (idx: number, dir: -1 | 1) => {
    // Reorder within activePaths
    const active = activePaths;
    const newIdx = idx + dir;
    if (newIdx < 0 || newIdx >= active.length) return;
    // Swap priorities
    const updated = [...activePaths];
    const tmp = updated[idx].priority;
    updated[idx] = { ...updated[idx], priority: updated[newIdx].priority };
    updated[newIdx] = { ...updated[newIdx], priority: tmp };
    // Map back
    setPaths((prev) => {
      let ai = 0;
      return prev.map((p) => {
        if (p.deleted) return p;
        const up = updated[ai++];
        return { ...p, priority: up.priority };
      });
    });
  };

  const testAll = async () => {
    setTesting(true);
    setTestMsg("Testing…");
    try {
      const results: PathTestResult[] = await api.testAllPaths(printerId);
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

  const switchBack = async () => {
    try {
      await api.switchBackToPath0(printerId);
      const ap = await api.getActivePath(printerId);
      setActivePath(ap);
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      setSavedMsg(`Switch failed: ${msg}`);
    }
  };

  const save = async () => {
    setSaving(true);
    setSavedMsg("");
    try {
      // Update printer settings.
      await api.updatePrinter(printerId, {
        fail_after_checks: failAfter,
        return_after_secs: returnAfter,
        hardened_nozzle: hardenedNozzle,
        ...(printer.firmware === "bambu"
          ? { access_code: authValue }
          : { api_key: authValue }),
      });

      // Sync paths: delete removed, update changed, create new.
      for (const p of paths) {
        if (p.deleted && p.id != null) {
          await api.deletePath(printerId, p.id);
        } else if (!p.deleted && p.id == null) {
          await api.createPath(printerId, { name: p.name, url: p.url, priority: p.priority });
        } else if (!p.deleted && p.id != null) {
          await api.updatePath(printerId, p.id, {
            name: p.name,
            url: p.url,
            priority: p.priority,
          });
        }
      }

      // Reload paths from server.
      const updated = await api.listPaths(printerId);
      setPaths(
        updated.map((p) => ({
          id: p.id,
          name: p.name,
          url: p.url,
          priority: p.priority,
          healthy: null,
          latency_ms: null,
        }))
      );

      await sshRef.current?.save();

      setSavedMsg("Saved");
      setTimeout(() => setSavedMsg(""), 3000);
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      setSavedMsg(`Error: ${msg}`);
    } finally {
      setSaving(false);
    }
  };

  // Summary chip label + colour.
  const activeIdx = activePath?.index ?? 0;
  const isOnPrimary = activeIdx === 0;
  const allDown = activePaths.length > 0 && activePaths.every((p) => p.healthy === false);
  let summaryLabel = isOnPrimary ? `on primary${activePath ? ` · ${activePath.path.name}` : ""}` : `on backup ${activeIdx + 1}${activePath ? ` · ${activePath.path.name}` : ""}`;
  if (allDown) summaryLabel = "all paths down";
  const summaryColor = allDown
    ? "var(--fw-danger)"
    : isOnPrimary
      ? "var(--fw-success)"
      : "var(--fw-warning)";

  const authLabel = printer.firmware === "bambu" ? "Access code" : "API key (optional)";
  const authHelp = printer.firmware === "bambu"
    ? "8-digit LAN access code from the printer's settings."
    : "Moonraker API key. Leave blank if no auth is configured.";

  const capsList: { k: string; v: string }[] = caps
    ? [
        caps.firmware_version ? { k: "Firmware", v: caps.firmware_version } : null,
        caps.hostname ? { k: "Hostname", v: caps.hostname } : null,
        caps.bed_size ? { k: "Bed", v: `${caps.bed_size[0]} × ${caps.bed_size[1]} mm` } : null,
        { k: "Extruders", v: String(caps.extruder_count) },
        caps.nozzle_diameter ? { k: "Nozzle", v: `${caps.nozzle_diameter} mm` } : null,
        caps.printer_model ? { k: "Model", v: caps.printer_model } : null,
      ].filter(Boolean) as { k: string; v: string }[]
    : [];

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
    <div role="tabpanel" aria-label="Config" className="printer-config">
      <div className="printer-config__bar">
        <h2 className="printer-config__title">Printer settings</h2>
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
        <button type="button" className="printer-config__save" onClick={save} disabled={saving}>
          {saving ? "Saving…" : "Save changes"}
        </button>
      </div>

      <div className="printer-config__cols">
      {/* ── Connection section ───────────────────────────────────── */}
      <section
        style={{
          border: "var(--fw-border-width) solid var(--fw-border)",
          background: "var(--fw-raised)",
          padding: "var(--fw-space-5)",
          display: "flex",
          flexDirection: "column",
          gap: 14,
        }}
      >
        <h2 style={{ margin: 0, fontSize: 18 }}>Connection</h2>

        <div style={{ display: "flex", flexDirection: "column", gap: "var(--fw-space-2)" }}>
          {/* Header row */}
          <div style={{ display: "flex", flexWrap: "wrap", alignItems: "center", gap: "var(--fw-space-2)" }}>
            <span style={{ flex: 1, fontSize: 13, fontWeight: "bold" }}>
              Connection paths{" "}
              <span style={{ fontWeight: "normal", color: "var(--fw-muted)" }}>
                · tried top to bottom
              </span>
            </span>
            <span
              style={{
                fontFamily: "var(--fw-mono)",
                fontSize: 11,
                color: summaryColor,
              }}
            >
              ● {summaryLabel}
            </span>
          </div>

          {/* Path rows */}
          {activePaths.map((p, i) => {
            const pathIdx = paths.indexOf(p);
            const statusLabel =
              p.healthy === null
                ? "STANDBY"
                : p.healthy
                  ? `ACTIVE · ${p.latency_ms ?? "?"}ms`
                  : "DOWN";
            const statusFg =
              p.healthy === null
                ? "var(--fw-muted)"
                : p.healthy
                  ? "var(--fw-success)"
                  : "var(--fw-danger)";
            const statusBg =
              p.healthy === null
                ? "var(--fw-surface)"
                : p.healthy
                  ? "color-mix(in srgb, var(--fw-success) 12%, transparent)"
                  : "color-mix(in srgb, var(--fw-danger) 12%, transparent)";

            return (
              <div
                key={pathIdx}
                className="printer-config__path"
                style={{
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
                    minHeight: "var(--fw-control-h-sm)",
                    border: "var(--fw-border-width) solid var(--fw-muted)",
                    background: "var(--fw-raised)",
                    padding: "0 var(--fw-space-2)",
                    borderRadius: "var(--fw-radius)",
                    boxSizing: "border-box",
                    width: "100%",
                    fontSize: 13,
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
                  aria-label={`Path ${i + 1} address`}
                  style={{
                    minHeight: "var(--fw-control-h-sm)",
                    border: "var(--fw-border-width) solid var(--fw-muted)",
                    background: "var(--fw-raised)",
                    padding: "0 var(--fw-space-2)",
                    borderRadius: "var(--fw-radius)",
                    boxSizing: "border-box",
                    width: "100%",
                    fontFamily: "var(--fw-mono)",
                    fontSize: 12,
                  }}
                />
                <span
                  style={{
                    fontFamily: "var(--fw-mono)",
                    fontSize: 11,
                    fontWeight: "bold",
                    padding: "3px 6px",
                    textAlign: "center",
                    color: statusFg,
                    background: statusBg,
                  }}
                >
                  {statusLabel}
                </span>
                <span style={{ display: "flex", gap: "var(--fw-space-1)" }}>
                  <button
                    type="button"
                    onClick={() => movePath(i, -1)}
                    disabled={i === 0}
                    aria-label="Move up"
                    style={{
                      width: 32,
                      height: 32,
                      border: "var(--fw-border-width) solid var(--fw-border)",
                      background: "transparent",
                      cursor: "pointer",
                      borderRadius: "var(--fw-radius)",
                      color: "var(--fw-text)",
                    }}
                  >
                    ↑
                  </button>
                  <button
                    type="button"
                    onClick={() => movePath(i, 1)}
                    disabled={i === activePaths.length - 1}
                    aria-label="Move down"
                    style={{
                      width: 32,
                      height: 32,
                      border: "var(--fw-border-width) solid var(--fw-border)",
                      background: "transparent",
                      cursor: "pointer",
                      borderRadius: "var(--fw-radius)",
                      color: "var(--fw-text)",
                    }}
                  >
                    ↓
                  </button>
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
                      color: "var(--fw-text)",
                    }}
                  >
                    ✕
                  </button>
                </span>
              </div>
            );
          })}

          {/* Add + Test all */}
          <div style={{ display: "flex", flexWrap: "wrap", gap: "var(--fw-space-2)", alignItems: "center" }}>
            <button
              type="button"
              onClick={addPath}
              style={{
                minHeight: "var(--fw-control-h-sm)",
                padding: "0 var(--fw-space-3)",
                border: "var(--fw-border-width) dashed var(--fw-muted)",
                background: "transparent",
                color: "var(--fw-text)",
                borderRadius: "var(--fw-radius)",
                fontSize: 13,
                cursor: "pointer",
              }}
            >
              + Add backup path
            </button>
            <button
              type="button"
              onClick={testAll}
              disabled={testing}
              style={{
                minHeight: "var(--fw-control-h-sm)",
                padding: "0 var(--fw-space-3)",
                border: "var(--fw-border-width) solid var(--fw-muted)",
                background: "transparent",
                color: "var(--fw-text)",
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

          {/* Failover settings */}
          <div
            style={{
              display: "grid",
              gridTemplateColumns: "repeat(auto-fit, minmax(200px, 1fr))",
              gap: "var(--fw-space-2)",
            }}
          >
            <label style={{ display: "flex", flexDirection: "column", gap: "var(--fw-space-1)" }}>
              <span style={{ fontSize: 12, fontWeight: "bold" }}>
                Switch to the next path after
              </span>
              <select
                value={failAfter}
                onChange={(e) => setFailAfter(Number(e.target.value))}
                style={{
                  minHeight: "var(--fw-control-h-sm)",
                  border: "var(--fw-border-width) solid var(--fw-muted)",
                  background: "var(--fw-raised)",
                  padding: "0 var(--fw-space-2)",
                  borderRadius: "var(--fw-radius)",
                }}
              >
                {FAIL_OPTIONS.map((o) => (
                  <option key={o.value} value={o.value}>
                    {o.label}
                  </option>
                ))}
              </select>
            </label>
            <label style={{ display: "flex", flexDirection: "column", gap: "var(--fw-space-1)" }}>
              <span style={{ fontSize: 12, fontWeight: "bold" }}>
                Go back up when a higher path is
              </span>
              <select
                value={returnAfter}
                onChange={(e) => setReturnAfter(Number(e.target.value))}
                style={{
                  minHeight: "var(--fw-control-h-sm)",
                  border: "var(--fw-border-width) solid var(--fw-muted)",
                  background: "var(--fw-raised)",
                  padding: "0 var(--fw-space-2)",
                  borderRadius: "var(--fw-radius)",
                }}
              >
                {RETURN_OPTIONS.map((o) => (
                  <option key={o.value} value={o.value}>
                    {o.label}
                  </option>
                ))}
              </select>
            </label>
          </div>

          {/* Switch back now button */}
          {activePath != null && activePath.index > 0 && (
            <div style={{ display: "flex", alignItems: "center", gap: "var(--fw-space-3)" }}>
              <button
                type="button"
                onClick={switchBack}
                style={{
                  minHeight: "var(--fw-control-h-sm)",
                  padding: "0 var(--fw-space-3)",
                  border: "var(--fw-border-width) solid var(--fw-muted)",
                  background: "transparent",
                  borderRadius: "var(--fw-radius)",
                  cursor: "pointer",
                  fontSize: 13,
                }}
              >
                Switch back to primary now
              </button>
            </div>
          )}

          <span style={{ fontSize: 12, color: "var(--fw-muted)" }}>
            A switch shows on the printer card and in Attention. Running jobs keep going; only Foreman's connection moves.
          </span>
        </div>

        {/* Auth field */}
        <label style={{ display: "flex", flexDirection: "column", gap: "var(--fw-space-2)" }}>
          <span style={{ fontSize: 13, fontWeight: "bold" }}>{authLabel}</span>
          <input
            type="password"
            value={authValue}
            onChange={(e) => setAuthValue(e.target.value)}
            style={{
              minHeight: "var(--fw-control-h)",
              border: "var(--fw-border-width) solid var(--fw-muted)",
              background: "var(--fw-raised)",
              padding: "0 var(--fw-space-3)",
              borderRadius: "var(--fw-radius)",
            }}
          />
          <span style={{ fontSize: 12, color: "var(--fw-muted)" }}>{authHelp}</span>
        </label>
      </section>

      <SshSettings ref={sshRef} printerId={printerId} firmware={printer.firmware} />

      {/* Display (browser-local, applies immediately) */}
      <section
        aria-label="Display"
        style={{
          border: "var(--fw-border-width) solid var(--fw-border)",
          background: "var(--fw-raised)",
          padding: "var(--fw-space-5)",
          display: "flex",
          flexDirection: "column",
          gap: "var(--fw-space-3)",
        }}
      >
        <h2 style={{ margin: 0, fontSize: 18 }}>Display</h2>
        <label style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <span style={{ fontSize: 13, fontWeight: "bold" }}>
            Webcam overlay opacity{" "}
            <span style={{ fontFamily: "var(--fw-mono)", fontWeight: "normal", color: "var(--fw-muted)" }}>
              {Math.round(overlayAlpha * 100)}%
            </span>
          </span>
          <input
            type="range"
            min={0.2}
            max={1}
            step={0.05}
            value={overlayAlpha}
            onChange={(e) => onOverlayAlpha(Number(e.target.value))}
            style={{ accentColor: "var(--fw-accent-hover)" }}
          />
          <span style={{ fontSize: 12, color: "var(--fw-muted)" }}>
            How see-through the Console and Macros panels are over the webcam. Applies immediately and is saved in
            this browser.
          </span>
        </label>
      </section>

      {/* ── Capabilities section ─────────────────────────────────── */}
      <section
        style={{
          border: "var(--fw-border-width) solid var(--fw-border)",
          background: "var(--fw-raised)",
          padding: "var(--fw-space-5)",
          display: "flex",
          flexDirection: "column",
          gap: "var(--fw-space-3)",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: "var(--fw-space-3)" }}>
          <h2 style={{ flex: 1, margin: 0, fontSize: 18 }}>Capabilities</h2>
        </div>
        {capsList.length === 0 ? (
          <p style={{ margin: 0, fontSize: 13, color: "var(--fw-muted)" }}>
            Capabilities not yet probed.
          </p>
        ) : (
          capsList.map((c) => (
            <div
              key={c.k}
              style={{
                display: "flex",
                justifyContent: "space-between",
                gap: "var(--fw-space-3)",
                minHeight: "var(--fw-control-h-md)",
                alignItems: "center",
                borderBottom: "var(--fw-border-width) solid var(--fw-border)",
              }}
            >
              <span>{c.k}</span>
              <span style={{ fontFamily: "var(--fw-mono)" }}>{c.v}</span>
            </div>
          ))
        )}
        <label style={{ display: "flex", alignItems: "center", gap: 10, minHeight: "var(--fw-control-h)" }}>
          <input
            type="checkbox"
            checked={hardenedNozzle}
            onChange={(e) => setHardenedNozzle(e.target.checked)}
            style={{ width: 20, height: 20, accentColor: "var(--fw-accent-hover)" }}
          />
          Hardened nozzle (abrasive filaments allowed)
        </label>
      </section>
      </div>
    </div>
  );
}
