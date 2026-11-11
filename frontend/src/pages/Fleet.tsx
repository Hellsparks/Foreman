import {
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { useOutletContext, useNavigate } from "react-router-dom";
import { api } from "../api/client";
import { queue } from "../api/queue";
import { FAIL_REASONS, jobs, type JobOutcome } from "../api/jobs";
import { useEventStream } from "../hooks/useEventStream";
import type { PrinterWithStatus, AttentionItem } from "../api/client";
import type { PrinterStatus } from "../api/types/PrinterStatus";
import type { FirmwareKind } from "../api/types/FirmwareKind";
import type { ShellContext } from "../shell/Shell";
import { SPOOL_TOPICS, spools, type LoadedSlot } from "../api/spools";
import LoadSpoolPanel from "../filament/LoadSpoolPanel";
import { StatusChip, ProgressBar, Swatch } from "../ui";
import type { Tone } from "../ui";
import { useSignedUrl } from "../hooks/useSignedUrl";
import { ATTENTION_PREFIX, everyText, maintenanceApi, type TaskTemplate } from "../api/maintenance";
import "./Fleet.css";

// ── helpers ──────────────────────────────────────────────────────────────────

function fwLabel(fw: string): string {
  return fw.toUpperCase();
}

/** Ensure a URL has a protocol prefix (bare IPs become http://). */
function ensureProtocol(url: string): string {
  return /^https?:\/\//i.test(url) ? url : `http://${url}`;
}

interface ChipData {
  hex: string | undefined;
  label: string;
  empty: boolean;
}

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

function formatSeconds(secs: number): string {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (h > 0) return `${h} h ${m} m`;
  return `${m} m`;
}

function jobSubLine(status: PrinterStatus | null | undefined): string {
  if (!status) return "";
  if (status.remaining_time != null && status.remaining_time > 0) {
    const eta = new Date(Date.now() + status.remaining_time * 1000);
    const hh = eta.getHours().toString().padStart(2, "0");
    const mm = eta.getMinutes().toString().padStart(2, "0");
    return `${formatSeconds(status.remaining_time)} left · ends ${hh}:${mm}`;
  }
  if (status.print_duration != null && status.print_duration > 0) {
    return `Running ${formatSeconds(status.print_duration)}`;
  }
  return "";
}

function tempsLine(status: PrinterStatus | null | undefined): string {
  if (!status) return "—";
  const parts: string[] = [];
  if (status.nozzle) {
    const t = status.nozzle.target > 0
      ? ` / ${status.nozzle.target.toFixed(0)}`
      : "";
    parts.push(`Nozzle ${status.nozzle.actual.toFixed(0)}${t} °C`);
  }
  if (status.bed) {
    const t = status.bed.target > 0
      ? ` / ${status.bed.target.toFixed(0)}`
      : "";
    parts.push(`Bed ${status.bed.actual.toFixed(0)}${t} °C`);
  }
  return parts.length > 0 ? parts.join(" · ") : "—";
}

function amsChips(
  status: PrinterStatus | null | undefined,
  fw: string,
  slots: LoadedSlot[] = [],
): ChipData[] {
  // Klipper: Moonraker's active spool (task 4.1).
  const klipper = slots.filter((s) => s.spool_id != null || s.writable);
  if (fw !== "bambu" && klipper.length) {
    return klipper.map((s) => ({
      hex: s.spool?.color_hex ? `#${s.spool.color_hex}` : undefined,
      label: s.spool
        ? `${s.label} #${s.spool.id} ${s.spool.material ?? ""} ${Math.round(s.spool.remaining_weight ?? 0)} g`.replace(/\s+/g, " ")
        : s.spool_id != null
          ? `${s.label} #${s.spool_id}`
          : `${s.label} no spool`,
      empty: !s.spool?.color_hex,
    }));
  }
  if (!status) return [];
  if (fw === "bambu" && status.ams_trays && status.ams_trays.length > 0) {
    return status.ams_trays
      .filter((t) => t.slot < 254)
      .map((t, i) => ({
        hex: t.color ? `#${t.color}` : undefined,
        label: `AMS${i + 1} ${t.material ?? "?"}`,
        empty: !t.color,
      }));
  }
  return [];
}

function attentionTone(kind: string): Tone {
  if (kind === "error") return "bad";
  if (kind === "print_finished") return "warn";
  if (kind === "offline") return "off";
  if (kind === "paused") return "warn";
  if (kind.startsWith(ATTENTION_PREFIX)) return "warn";
  return "warn";
}

function attentionSymbol(kind: string): string {
  if (kind === "error") return "✖";
  if (kind === "print_finished") return "◐";
  if (kind === "offline") return "○";
  if (kind === "paused") return "‖";
  if (kind.startsWith(ATTENTION_PREFIX)) return "⚙";
  return "!";
}

const ATTENTION_TOPICS = ["attention.added", "attention.updated", "attention.cleared"];

// ── Fleet page ───────────────────────────────────────────────────────────────

export default function Fleet() {
  const navigate = useNavigate();
  const {
    statuses,
    connections,
    openInspector,
    closeInspector,
    inspectorOpen,
    inspectorBodyId,
  } = useOutletContext<ShellContext>();

  const [printers, setPrinters] = useState<PrinterWithStatus[]>([]);
  const [loading, setLoading] = useState(true);
  const [attentionItems, setAttentionItems] = useState<AttentionItem[]>([]);
  const [attnOpen, setAttnOpen] = useState(true);
  /** Per attention item: the rating given here, and whether the reason picker is open. */
  const [rated, setRated] = useState<Record<number, { outcome: JobOutcome; reason?: string }>>({});
  const [reasonFor, setReasonFor] = useState<number | null>(null);
  const [menuFor, setMenuFor] = useState<number | null>(null);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [showAddPanel, setShowAddPanel] = useState(false);
  const [slots, setSlots] = useState<LoadedSlot[]>([]);
  const [loadFor, setLoadFor] = useState<number | null>(null);

  // Snapshot timestamps for cache-busting (per printer).
  const [snapTs, setSnapTs] = useState<Record<number, number>>({});
  const snapInterval = useRef<ReturnType<typeof setInterval> | null>(null);

  const loadPrinters = useCallback(async () => {
    try {
      const list = await api.listPrinters();
      setPrinters(list);
    } catch (e) {
      console.error("Failed to load printers:", e);
    } finally {
      setLoading(false);
    }
  }, []);

  const loadAttention = useCallback(async () => {
    try {
      const items = await api.listAttention();
      setAttentionItems(items);
    } catch (e) {
      console.error("Failed to load attention:", e);
    }
  }, []);

  const loadSlots = useCallback(async () => {
    try {
      setSlots(await spools.loaded());
    } catch (e) {
      console.error("Failed to load spools:", e);
    }
  }, []);

  useEffect(() => {
    loadPrinters();
    loadAttention();
    loadSlots();
  }, [loadPrinters, loadAttention, loadSlots]);

  const onSpoolEvent = useCallback(() => void loadSlots(), [loadSlots]);
  useEventStream(onSpoolEvent, SPOOL_TOPICS);

  // Live Attention strip (e.g. "clear the plate" when a print finishes).
  const onAttentionEvent = useCallback(() => void loadAttention(), [loadAttention]);
  useEventStream(onAttentionEvent, ATTENTION_TOPICS);

  // Refresh snapshots every 10 s.
  useEffect(() => {
    snapInterval.current = setInterval(() => {
      setSnapTs((prev) => {
        const next = { ...prev };
        printers.forEach((p) => { next[p.id] = Date.now(); });
        return next;
      });
    }, 10_000);
    return () => {
      if (snapInterval.current) clearInterval(snapInterval.current);
    };
  }, [printers]);

  // Close inspector when navigating away.
  useEffect(() => {
    return () => closeInspector();
  }, [closeInspector]);

  // Close menu on click-outside.
  useEffect(() => {
    if (menuFor === null) return;
    const handler = () => setMenuFor(null);
    window.addEventListener("click", handler);
    return () => window.removeEventListener("click", handler);
  }, [menuFor]);

  // Live status merged with REST snapshot.
  const liveStatus = (p: PrinterWithStatus): PrinterStatus | null => {
    return (statuses[p.id] as PrinterStatus | undefined) ?? p.status ?? null;
  };

  const selectPrinter = (id: number) => {
    setSelectedId(id);
    setLoadFor(null);
    openInspector();
  };

  const deselectPrinter = () => {
    setSelectedId(null);
    setLoadFor(null);
    closeInspector();
  };

  const startLoad = (id: number) => {
    setSelectedId(id);
    setLoadFor(id);
    setMenuFor(null);
    openInspector();
  };

  // Summary line.
  const summaryLine = (): string => {
    const count = printers.length;
    if (count === 0) return "No printers";
    const printing = printers.filter(
      (p) => (liveStatus(p)?.state ?? "offline") === "printing"
    ).length;
    const error = printers.filter(
      (p) => (liveStatus(p)?.state ?? "offline") === "error"
    ).length;
    const offline = printers.filter(
      (p) =>
        (connections[p.id] === "offline") ||
        (liveStatus(p)?.state === "offline")
    ).length;
    const parts: string[] = [`${count} printer${count !== 1 ? "s" : ""}`];
    if (printing) parts.push(`${printing} printing`);
    if (error) parts.push(`${error} error${error !== 1 ? "s" : ""}`);
    if (offline) parts.push(`${offline} offline`);
    return parts.join(" · ");
  };

  const handleAdd = async (data: AddPrinterData) => {
    try {
      const created = await api.createPrinter({
        name: data.name,
        firmware: data.firmware,
        primary_url: data.url || null,
        api_key: null,
        serial_number: data.serial || null,
        access_code: data.accessCode || null,
        tags: [],
        hardened_nozzle: false,
        slicer_profiles: [],
        sort_order: null,
        fail_after_checks: null,
        return_after_secs: null,
      });
      // Common service tasks the user kept ticked; a new printer starts at zero.
      if (data.templates.length > 0) {
        try {
          await maintenanceApi.fromTemplates(Number(created.id), data.templates, true);
        } catch (e) {
          console.error("Failed to add service tasks:", e);
        }
      }
      setShowAddPanel(false);
      loadPrinters();
    } catch (e) {
      console.error("Failed to add printer:", e);
    }
  };

  const plateCleared = async (printerId: number, itemId: number) => {
    try {
      await queue.plateCleared(printerId);
      setAttentionItems((prev) => prev.filter((i) => i.id !== itemId));
    } catch (e) {
      console.error("Failed to mark plate cleared:", e);
    }
  };

  const rateJob = async (item: AttentionItem, outcome: JobOutcome, reason?: string) => {
    if (item.job_id == null) return;
    try {
      await jobs.rate(item.job_id, outcome, reason ?? null);
      setRated((prev) => ({ ...prev, [item.id]: { outcome, reason } }));
      setReasonFor(outcome === "failed" && !reason ? item.id : null);
    } catch (e) {
      console.error("Failed to rate job:", e);
    }
  };

  const dismissAttention = async (id: number) => {
    try {
      await api.dismissAttention(id);
      setAttentionItems((prev) => prev.filter((i) => i.id !== id));
    } catch (e) {
      console.error("Failed to dismiss attention:", e);
    }
  };

  // Portal target for inspector content.
  const [portalTarget, setPortalTarget] = useState<Element | null>(null);
  useEffect(() => {
    // The inspector body div is rendered by Shell; find it once it's in the DOM.
    const el = document.getElementById(inspectorBodyId);
    setPortalTarget(el);
  }, [inspectorBodyId, inspectorOpen]);

  const selectedPrinter = printers.find((p) => p.id === selectedId) ?? null;

  return (
    <div className="fleet">
      {/* ── Header ──────────────────────────────────────────────────────── */}
      <div className="fleet__header">
        <div>
          <h1 className="fleet__title">Fleet</h1>
          <p className="fleet__summary">{summaryLine()}</p>
        </div>
        <div className="fleet__header-actions">
          <button
            type="button"
            className="fleet__add-btn"
            onClick={() => setShowAddPanel((v) => !v)}
          >
            {showAddPanel ? "Cancel" : "Add printer"}
          </button>
        </div>
      </div>

      {/* ── Add printer panel ────────────────────────────────────────────── */}
      {showAddPanel && (
        <div className="fleet__add-panel">
          <AddPrinterForm onSubmit={handleAdd} />
        </div>
      )}

      {/* ── Attention strip ──────────────────────────────────────────────── */}
      <section
        className="fleet__attention"
        aria-label="Attention"
      >
        <div className="fleet__attention-header">
          <h2 className="fleet__attention-title">
            Attention · {attentionItems.length}
          </h2>
          <div style={{ flex: 1 }} />
          <button
            type="button"
            className="fleet__attention-toggle"
            aria-expanded={attnOpen}
            onClick={() => setAttnOpen((v) => !v)}
          >
            {attnOpen ? "Collapse" : "Expand"}
          </button>
        </div>

        {attnOpen && (
          <>
            {attentionItems.length === 0 ? (
              <p className="fleet__attention-empty">
                ✓ Nothing needs you right now.
              </p>
            ) : (
              attentionItems.map((item) => {
                const tone = attentionTone(item.kind);
                const sym = attentionSymbol(item.kind);
                const relatedPrinter = item.printer_id != null
                  ? printers.find((p) => p.id === item.printer_id)
                  : null;
                return (
                  <div key={item.id} className="fleet__attn-row">
                    <StatusChip tone={tone} symbol={sym} word={item.kind === "offline" ? "Offline" : item.kind === "error" ? "Error" : item.kind === "print_finished" ? "Clear plate" : item.kind.startsWith(ATTENTION_PREFIX) ? "Service" : item.kind} />
                    <div className="fleet__attn-text">
                      <div className="fleet__attn-title">{item.title}</div>
                      {item.body && (
                        <div className="fleet__attn-body">{item.body}</div>
                      )}
                      {item.kind === "print_finished" && item.job_id != null && (
                        <div className="fleet__attn-rate" role="group" aria-label="How did it go?">
                          {rated[item.id] ? (
                            <span className="fleet__attn-rated">
                              Rated {rated[item.id].outcome === "good" ? "✓ Good" : `✖ Failed${rated[item.id].reason ? ` · ${rated[item.id].reason}` : ""}`}
                            </span>
                          ) : (
                            <span className="fleet__attn-rated">How did it go?</span>
                          )}
                          <button
                            type="button"
                            className="fleet__attn-chip"
                            aria-pressed={rated[item.id]?.outcome === "good"}
                            onClick={() => void rateJob(item, "good")}
                          >
                            ✓ Good
                          </button>
                          <button
                            type="button"
                            className="fleet__attn-chip"
                            aria-pressed={rated[item.id]?.outcome === "failed"}
                            onClick={() => void rateJob(item, "failed")}
                          >
                            ✖ Failed
                          </button>
                          {reasonFor === item.id &&
                            FAIL_REASONS.map((r) => (
                              <button
                                key={r}
                                type="button"
                                className="fleet__attn-chip fleet__attn-chip--reason"
                                onClick={() => void rateJob(item, "failed", r)}
                              >
                                {r}
                              </button>
                            ))}
                        </div>
                      )}
                    </div>
                    <div className="fleet__attn-actions">
                      {item.kind === "print_finished" && item.printer_id != null && (
                        <button
                          type="button"
                          className="fleet__attn-btn fleet__attn-btn--primary"
                          onClick={() => void plateCleared(item.printer_id as number, item.id)}
                        >
                          Plate cleared
                        </button>
                      )}
                      {item.kind.startsWith(ATTENTION_PREFIX) && (
                        <button
                          type="button"
                          className="fleet__attn-btn fleet__attn-btn--primary"
                          onClick={() => navigate(`/service?task=${item.kind.slice(ATTENTION_PREFIX.length)}`)}
                        >
                          Open in Service
                        </button>
                      )}
                      {relatedPrinter && (
                        <button
                          type="button"
                          className="fleet__attn-btn"
                          onClick={() => selectPrinter(relatedPrinter.id)}
                        >
                          Open printer
                        </button>
                      )}
                      {/* A service item clears when the task is marked done. */}
                      {!item.kind.startsWith(ATTENTION_PREFIX) && (
                        <button
                          type="button"
                          className="fleet__attn-btn"
                          onClick={() => dismissAttention(item.id)}
                        >
                          Dismiss
                        </button>
                      )}
                    </div>
                  </div>
                );
              })
            )}
          </>
        )}
      </section>

      {/* ── Printer grid ─────────────────────────────────────────────────── */}
      {loading ? (
        <p className="fleet__loading">Loading printers…</p>
      ) : printers.length === 0 ? (
        <p className="fleet__empty">No printers registered. Add one to get started.</p>
      ) : (
        <section className="fleet__grid" aria-label="Printers">
          {printers.map((p) => {
            const st = liveStatus(p);
            const state = st?.state ?? "offline";
            const isSelected = selectedId === p.id;
            const ts = snapTs[p.id] ?? 0;
            return (
              <PrinterCard
                key={p.id}
                printer={p}
                slots={slots.filter((s) => s.printer_id === p.id)}
                onLoadSpool={() => startLoad(p.id)}
                status={st}
                state={state}
                selected={isSelected}
                menuOpen={menuFor === p.id}
                snapshotTs={ts}
                onSelect={() => selectPrinter(p.id)}
                onMenuToggle={(e) => {
                  e.stopPropagation();
                  setMenuFor((prev) => (prev === p.id ? null : p.id));
                }}
                onOpenPage={() => navigate(`/printer/${p.id}`)}
              />
            );
          })}
        </section>
      )}

      {/* ── Inspector portal ─────────────────────────────────────────────── */}
      {portalTarget && selectedPrinter && inspectorOpen &&
        createPortal(
          loadFor === selectedPrinter.id ? (
            <LoadSpoolPanel
              printerId={selectedPrinter.id}
              onClose={() => setLoadFor(null)}
              onLoaded={() => void loadSlots()}
            />
          ) : (
            <PrinterInspector
              printer={selectedPrinter}
              status={liveStatus(selectedPrinter)}
              slots={slots.filter((s) => s.printer_id === selectedPrinter.id)}
              onLoadSpool={() => startLoad(selectedPrinter.id)}
              onClose={deselectPrinter}
              onOpenPage={() => navigate(`/printer/${selectedPrinter.id}`)}
            />
          ),
          portalTarget
        )
      }
    </div>
  );
}

// ── PrinterCard ───────────────────────────────────────────────────────────────

function PrinterCard({
  printer,
  slots,
  onLoadSpool,
  status,
  state,
  selected,
  menuOpen,
  snapshotTs,
  onSelect,
  onMenuToggle,
  onOpenPage,
}: {
  printer: PrinterWithStatus;
  slots: LoadedSlot[];
  onLoadSpool: () => void;
  status: PrinterStatus | null;
  state: string;
  selected: boolean;
  menuOpen: boolean;
  snapshotTs: number;
  onSelect: () => void;
  onMenuToggle: (e: React.MouseEvent) => void;
  onOpenPage: () => void;
}) {
  const tone = statusTone(state);
  const pct =
    status?.progress != null ? Math.round(status.progress * 100) : 0;
  const hasProgress = pct > 0 && pct < 100;
  const chips = amsChips(status, printer.firmware, slots);
  const canLoad = slots.some((s) => s.writable);
  const jobName = status?.filename ?? (state === "offline" ? "Not reachable" : "Ready for next job");
  const sub = jobSubLine(status);
  const isOffline = state === "offline";
  const snapshotUrl = useSignedUrl(isOffline ? null : api.snapshotPath(printer.id, snapshotTs));

  return (
    <article
      className={`fleet-card${selected ? " fleet-card--selected" : ""}${isOffline ? " fleet-card--offline" : ""}`}
    >
      {/* Row 1: name + firmware tag + backup path label + menu */}
      <div className="fleet-card__toprow">
        <button
          type="button"
          className="fleet-card__name"
          onClick={onSelect}
        >
          {printer.name}
        </button>
        <span className="fleet-card__fw">{fwLabel(printer.firmware)}</span>
        {printer.active_path_index != null && printer.active_path_index > 0 && printer.active_path_name && (
          <span
            className="fleet-card__path-label"
            title={`Connected via backup path: ${printer.active_path_name}`}
            style={{ fontSize: 11, color: "var(--fw-warning)", fontFamily: "var(--fw-mono)" }}
          >
            via {printer.active_path_name}
          </span>
        )}
        <div style={{ flex: 1 }} />
        <button
          type="button"
          className="fleet-card__menu-btn"
          aria-label={`Actions for ${printer.name}`}
          aria-expanded={menuOpen}
          onClick={onMenuToggle}
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
            <circle cx="5" cy="12" r="2" />
            <circle cx="12" cy="12" r="2" />
            <circle cx="19" cy="12" r="2" />
          </svg>
        </button>
      </div>

      {/* Row 2: status + thumbnail */}
      <div className="fleet-card__mid">
        <div className="fleet-card__status">
          <StatusChip tone={tone} symbol={statusSymbol(state)} word={statusWord(state)} />
          <div className="fleet-card__job">{jobName}</div>
          {sub && <div className="fleet-card__sub">{sub}</div>}
        </div>
        <CameraThumb url={snapshotUrl} />
      </div>

      {/* Progress bar */}
      {hasProgress && (
        <ProgressBar value={pct} />
      )}

      {/* Temps */}
      <div className="fleet-card__temps">{tempsLine(status)}</div>

      {/* Filament chips */}
      {chips.length > 0 && (
        <div className="fleet-card__chips">
          {chips.map((c, i) => (
            <span key={i} className="fleet-card__chip">
              <Swatch
                kind={c.empty ? "empty" : "solid"}
                color={c.hex}
                size={14}
              />
              {c.label}
            </span>
          ))}
        </div>
      )}

      {/* Card menu dropdown */}
      {menuOpen && (
        <div
          className="fleet-card__dropdown"
          role="group"
          aria-label={`Actions for ${printer.name}`}
          onClick={(e) => e.stopPropagation()}
        >
          <button
            type="button"
            className="fleet-card__dropdown-item"
            onClick={onOpenPage}
          >
            Open printer page
          </button>
          {canLoad && (
            <button type="button" className="fleet-card__dropdown-item" onClick={onLoadSpool}>
              Load spool…
            </button>
          )}
          {printer.primary_url && (
            <button
              type="button"
              className="fleet-card__dropdown-item"
              onClick={() => {
                window.open(ensureProtocol(printer.primary_url!), "_blank", "noopener,noreferrer");
              }}
            >
              Open native UI
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="square" strokeLinejoin="miter" aria-hidden="true" style={{ verticalAlign: "-2px", marginLeft: 6 }}>
                <path d="M14 4h6v6M20 4l-9 9M18 14v6H4V6h6" />
              </svg>
            </button>
          )}
        </div>
      )}
    </article>
  );
}

// ── CameraThumb ───────────────────────────────────────────────────────────────

function CameraThumb({ url }: { url: string | null }) {
  const [err, setErr] = useState(false);
  // Reset error when url changes.
  useEffect(() => setErr(false), [url]);

  return (
    <div className="fleet-card__cam" aria-hidden="true">
      {url && !err ? (
        <img
          src={url}
          alt=""
          className="fleet-card__cam-img"
          onError={() => setErr(true)}
        />
      ) : (
        <span className="fleet-card__cam-label">
          {url ? "NO SIGNAL" : "NO CAMERA"}
        </span>
      )}
    </div>
  );
}

// ── PrinterInspector (rendered via portal) ────────────────────────────────────

function PrinterInspector({
  printer,
  status,
  slots,
  onLoadSpool,
  onClose,
  onOpenPage,
}: {
  printer: PrinterWithStatus;
  status: PrinterStatus | null;
  slots: LoadedSlot[];
  onLoadSpool: () => void;
  onClose: () => void;
  onOpenPage: () => void;
}) {
  const state = status?.state ?? "offline";
  const tone = statusTone(state);
  const pct =
    status?.progress != null ? Math.round(status.progress * 100) : 0;
  const hasProgress = pct > 0 && pct < 100;
  const chips = amsChips(status, printer.firmware, slots);
  const canLoad = slots.some((s) => s.writable);
  const readOnly = slots.find((s) => !s.writable)?.read_only_reason;

  return (
    <div className="fleet-insp">
      {/* Header */}
      <div className="fleet-insp__header">
        <div className="fleet-insp__header-text">
          <div className="fleet-insp__type">
            Printer · {fwLabel(printer.firmware)}
          </div>
          <h2 className="fleet-insp__name">{printer.name}</h2>
          <StatusChip
            tone={tone}
            symbol={statusSymbol(state)}
            word={statusWord(state)}
          />
        </div>
        <button
          type="button"
          className="fleet-insp__close"
          aria-label="Close inspector"
          onClick={onClose}
        >
          ✕
        </button>
      </div>

      {/* Current job */}
      <div className="fleet-insp__section">
        <div className="fleet-insp__section-label">Current job</div>
        <div className="fleet-insp__job-name">
          {status?.filename ?? (state === "offline" ? "Not reachable" : "Ready for next job")}
        </div>
        {jobSubLine(status) && (
          <div className="fleet-insp__job-sub">{jobSubLine(status)}</div>
        )}
        {hasProgress && (
          <div style={{ marginTop: 8 }}>
            <ProgressBar value={pct} height={8} />
          </div>
        )}
      </div>

      {/* Temperatures */}
      <div className="fleet-insp__section">
        <div className="fleet-insp__section-label">Temperatures</div>
        <div className="fleet-insp__temps">{tempsLine(status)}</div>
      </div>

      {/* Loaded filament */}
      {(chips.length > 0 || canLoad) && (
        <div className="fleet-insp__section">
          <div className="fleet-insp__section-label">Loaded filament</div>
          {chips.map((c, i) => (
            <div key={i} className="fleet-insp__chip-row">
              <Swatch
                kind={c.empty ? "empty" : "solid"}
                color={c.hex}
                size={20}
              />
              <span className="fleet-insp__chip-label">{c.label}</span>
            </div>
          ))}
          {canLoad ? (
            <button type="button" className="fleet-insp__btn" style={{ marginTop: 8 }} onClick={onLoadSpool}>
              Load spool…
            </button>
          ) : (
            readOnly && <div className="fleet-insp__job-sub">{readOnly}</div>
          )}
        </div>
      )}

      <div style={{ flex: 1 }} />

      {/* Footer buttons */}
      <div className="fleet-insp__footer">
        <button
          type="button"
          className="fleet-insp__btn fleet-insp__btn--primary"
          onClick={onOpenPage}
        >
          Open printer page
        </button>
        {printer.primary_url && (
          <button
            type="button"
            className="fleet-insp__btn"
            onClick={() => {
              window.open(ensureProtocol(printer.primary_url!), "_blank", "noopener,noreferrer");
            }}
          >
            Native UI
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="square" strokeLinejoin="miter" aria-hidden="true" style={{ verticalAlign: "-2px", marginLeft: 6 }}>
              <path d="M14 4h6v6M20 4l-9 9M18 14v6H4V6h6" />
            </svg>
          </button>
        )}
      </div>
    </div>
  );
}

// ── AddPrinterForm ────────────────────────────────────────────────────────────

interface AddPrinterData {
  name: string;
  firmware: FirmwareKind;
  url: string;
  serial: string;
  accessCode: string;
  /** Maintenance template keys to add (5.3). */
  templates: string[];
}

function AddPrinterForm({ onSubmit }: { onSubmit: (data: AddPrinterData) => void }) {
  const [name, setName] = useState("");
  const [firmware, setFirmware] = useState<FirmwareKind>("moonraker");
  const [url, setUrl] = useState("");
  const [serial, setSerial] = useState("");
  const [accessCode, setAccessCode] = useState("");
  const isBambu = firmware === "bambu";
  const [templates, setTemplates] = useState<TaskTemplate[]>([]);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  useEffect(() => {
    maintenanceApi.templates().then(
      (t) => {
        setTemplates(t);
        setPicked(new Set(t.map((x) => x.key)));
      },
      () => setTemplates([]),
    );
  }, []);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) return;
    if (isBambu ? (!url.trim() || !serial.trim() || !accessCode.trim()) : !url.trim()) return;
    onSubmit({ name: name.trim(), firmware, url: url.trim(), serial: serial.trim(), accessCode: accessCode.trim(), templates: [...picked] });
  };

  return (
    <form className="fleet__form" onSubmit={handleSubmit}>
      <div className="fleet__form-row">
        <div className="fleet__form-field">
          <label className="fleet__form-label" htmlFor="add-name">Name</label>
          <input id="add-name" className="fleet__form-input" type="text" value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. HellV0" required />
        </div>
        <div className="fleet__form-field fleet__form-field--short">
          <label className="fleet__form-label" htmlFor="add-fw">Type</label>
          <select id="add-fw" className="fleet__form-input" value={firmware} onChange={(e) => setFirmware(e.target.value as FirmwareKind)}>
            <option value="moonraker">Moonraker</option>
            <option value="bambu">Bambu</option>
          </select>
        </div>
      </div>
      <div className="fleet__form-row">
        <div className="fleet__form-field">
          <label className="fleet__form-label" htmlFor="add-url">{isBambu ? "IP Address" : "Moonraker URL"}</label>
          <input id="add-url" className="fleet__form-input" type={isBambu ? "text" : "url"} value={url} onChange={(e) => setUrl(e.target.value)} placeholder={isBambu ? "10.0.0.249" : "http://10.0.0.177"} required />
        </div>
        {isBambu && (
          <>
            <div className="fleet__form-field">
              <label className="fleet__form-label" htmlFor="add-serial">Serial</label>
              <input id="add-serial" className="fleet__form-input" type="text" value={serial} onChange={(e) => setSerial(e.target.value)} placeholder="00W00A000000000" required />
            </div>
            <div className="fleet__form-field">
              <label className="fleet__form-label" htmlFor="add-access">Access code</label>
              <input id="add-access" className="fleet__form-input" type="password" value={accessCode} onChange={(e) => setAccessCode(e.target.value)} placeholder="8-digit code" required />
            </div>
          </>
        )}
        <button className="fleet__form-submit" type="submit">Add</button>
      </div>
      {templates.length > 0 && (
        <fieldset className="fleet__form-tasks">
          <legend className="fleet__form-label">Service tasks to start with (optional)</legend>
          {templates.map((t) => (
            <label key={t.key} className="fleet__form-task">
              <input
                type="checkbox"
                checked={picked.has(t.key)}
                onChange={(e) =>
                  setPicked((s) => {
                    const n = new Set(s);
                    if (e.target.checked) n.add(t.key);
                    else n.delete(t.key);
                    return n;
                  })
                }
              />
              <span>
                {t.name} <span className="fleet__form-task-every">· every {everyText(t)}</span>
              </span>
            </label>
          ))}
        </fieldset>
      )}
    </form>
  );
}
