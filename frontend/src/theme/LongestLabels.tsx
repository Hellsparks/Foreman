/**
 * "Longest labels" check (Settings → Appearance).
 *
 * Renders worst-case strings in the real components, in the current theme,
 * and flags anything that clips. A font or density change that breaks a
 * layout shows up here, and so do literals step 6.1 missed.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { Button, StatusChip, Swatch, SectionLabel } from "../ui";
import type { Tone } from "../ui";
import { PrinterCard } from "../pages/Fleet";
import SpoolSwatch from "../filament/SpoolSwatch";
import type { PrinterWithStatus } from "../api/client";
import type { PrinterStatus } from "../api/types/PrinterStatus";
import type { LoadedSlot } from "../api/spools";
import type { SpoolFacts } from "../api/types/SpoolFacts";
import { scanClipping, markHits, type ClipHit } from "./clipScan";
import { THEME_APPLIED_EVENT } from "./events";
import "../filament/filament.css";

// ── Worst-case data ──────────────────────────────────────────────────────────

export const LONG_PRINTER_NAMES = [
  "Voron 2.4 R2 350 — Workshop left bench",
  "Bambu Lab X1-Carbon Combo (garage)",
];
export const LONG_FILAMENT_NAMES = [
  "PolyTerra PLA+ Matte Muted Lavender Grey Gradient",
  "Galaxy Black Sparkle Silk Dual-Colour Coaxial",
];
const LONG_VENDOR = "Shenzhen Elegoo Technology Co. Ltd";
const LONG_FILE = "toolhead_kit_v3_stealthburner_cw2_plate_2_of_4_ABS-GF_0.2mm_6h12m.gcode";

const CHIPS: [Tone, string, string][] = [
  ["info", "▶", "Printing"],
  ["ok", "✓", "Finished"],
  ["warn", "‖", "Paused"],
  ["bad", "✖", "Error"],
  ["off", "○", "Offline"],
  ["off", "●", "Idle"],
  ["warn", "⚙", "Maintenance due"],
  ["warn", "◐", "Plate needs clearing"],
];

const ACTIONS = ["Open printer", "Reprint plate", "Clear plate", "Mark as done"];

function spool(id: number, name: string, hex: string, material: string): SpoolFacts {
  return {
    id, filament_id: id, name, vendor: LONG_VENDOR, material, modifier: null, abrasive: material.includes("CF"),
    color_hex: hex, multi_color_hexes: [], multi_color_direction: null, remaining_weight: 1000,
    initial_weight: 1000, used_weight: 0, spool_weight: 250, location: null, lot_nr: null, price: null,
    diameter: 1.75, extruder_temp: null, bed_temp: null, first_used: null, last_used: null,
    registered: "2026-01-01T00:00:00Z", archived: false, comment: null,
  };
}

const printerA = {
  id: -1, name: LONG_PRINTER_NAMES[0], firmware: "moonraker", active_path_index: 1,
  active_path_name: "Tailscale backup route", primary_url: null,
} as unknown as PrinterWithStatus;

const printerB = {
  id: -2, name: LONG_PRINTER_NAMES[1], firmware: "bambu", active_path_index: 0,
  active_path_name: null, primary_url: null,
} as unknown as PrinterWithStatus;

const slotsA = [0, 1].map((tool) => ({
  printer_id: -1, printer_name: printerA.name, tool, label: `T${tool}`, spool_id: 9000 + tool,
  spool: spool(9000 + tool, LONG_FILAMENT_NAMES[tool], tool ? "1B1B1B" : "B39DDB", "PETG-CF Rapid HF"),
  tray: null, writable: true, read_only_reason: null,
})) as LoadedSlot[];

const statusB: PrinterStatus = {
  state: "printing" as PrinterStatus["state"],
  nozzle: { actual: 285, target: 290 } as PrinterStatus["nozzle"],
  bed: { actual: 110, target: 110 } as PrinterStatus["bed"],
  progress: 0.42, filename: LONG_FILE, print_duration: 7200, display_message: null,
  layer: 120, total_layers: 300, remaining_time: 22_000, fans: null,
  ams_trays: [
    { slot: 0, color: "B39DDB", material: "PETG-CF Rapid HF", remaining: 80 },
    { slot: 1, color: "FFFFFF", material: "PLA Silk+", remaining: 50 },
    { slot: 2, color: "2E7D32", material: "ABS-GF", remaining: 20 },
    { slot: 3, color: null, material: null, remaining: null },
  ],
  hms_codes: [], print_end: null, print_message: null, filament_used_mm: null,
};

// ── Component ────────────────────────────────────────────────────────────────

const noop = () => {};

export default function LongestLabels() {
  const ref = useRef<HTMLDivElement>(null);
  const [hits, setHits] = useState<ClipHit[] | null>(null);
  const [pageHits, setPageHits] = useState<ClipHit[] | null>(null);

  const run = useCallback(async () => {
    await document.fonts.ready;
    // Two frames: let a theme's new values lay out first.
    await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
    const root = ref.current;
    if (!root) return;
    const h = scanClipping(root);
    markHits(root, h);
    setHits(h);
  }, []);

  useEffect(() => {
    void run();
    window.addEventListener(THEME_APPLIED_EVENT, run);
    window.addEventListener("resize", run);
    return () => {
      window.removeEventListener(THEME_APPLIED_EVENT, run);
      window.removeEventListener("resize", run);
    };
  }, [run]);

  const scanPage = () => {
    const shell = document.querySelector<HTMLElement>(".shell") ?? document.body;
    const h = scanClipping(shell).filter((x) => !ref.current?.contains(x.el));
    markHits(shell, [...h, ...(hits ?? [])]);
    setPageHits(h);
  };

  const clipped = hits?.filter((h) => h.kind === "clipped") ?? [];
  const truncated = hits?.filter((h) => h.kind === "truncated") ?? [];

  return (
    <div className="ll">
      <div className="ll__bar">
        <p className="ll__result" role="status" data-testid="ll-result">
          {hits === null
            ? "Checking…"
            : clipped.length === 0
              ? `✓ Nothing clips (${truncated.length} cut with … on purpose)`
              : `✖ ${clipped.length} clip${clipped.length === 1 ? "s" : ""} (${truncated.length} cut with … on purpose)`}
        </p>
        <Button variant="small" onClick={() => void run()}>Re-check</Button>
        <Button variant="small" onClick={scanPage}>Scan this page too</Button>
      </div>
      {clipped.length > 0 && <HitList hits={clipped} />}
      {pageHits && (
        <>
          <p className="ll__result">
            {pageHits.filter((h) => h.kind === "clipped").length === 0
              ? "✓ Nothing else on this page clips"
              : `✖ ${pageHits.filter((h) => h.kind === "clipped").length} clips elsewhere on this page`}
          </p>
          <HitList hits={pageHits.filter((h) => h.kind === "clipped")} />
        </>
      )}

      <div ref={ref} className="ll__stage" data-testid="ll-stage">
        <SectionLabel>Printer cards (narrowest grid column)</SectionLabel>
        <div className="ll__cards">
          <PrinterCard
            printer={printerA} slots={slotsA} onLoadSpool={noop} status={null} state="offline"
            selected={false} menuOpen={false} snapshotTs={0} onSelect={noop} onMenuToggle={noop} onOpenPage={noop}
          />
          <PrinterCard
            printer={printerB} slots={[]} onLoadSpool={noop} status={statusB} state="printing"
            selected={false} menuOpen={false} snapshotTs={0} onSelect={noop} onMenuToggle={noop} onOpenPage={noop}
          />
        </div>

        <SectionLabel>Status chips</SectionLabel>
        <div className="ll__row">
          {CHIPS.map(([tone, sym, word]) => (
            <StatusChip key={word} tone={tone} symbol={sym} word={word} />
          ))}
        </div>

        <SectionLabel>Action column (148 px buttons)</SectionLabel>
        <div className="ll__row">
          {ACTIONS.map((a) => (
            <Button key={a} fixedWidth={148}>{a}</Button>
          ))}
          <Button variant="primary">Send to printer</Button>
          <Button variant="small">Dismiss</Button>
        </div>

        <SectionLabel>Spool list</SectionLabel>
        <div className="fil-table-wrap">
          <table className="fil-table" style={{ minWidth: 0 }}>
            <tbody>
              {slotsA.map((s) => (
                <tr key={s.tool} className="fil-tr">
                  <td>
                    <button type="button" className="fil-row-btn">
                      <SpoolSwatch spool={s.spool} size={28} />
                      <span>
                        <b>#{s.spool?.id} {s.spool?.name}</b>
                        <br />
                        <span className="fil-sub">{s.spool?.vendor}</span>
                      </span>
                    </button>
                  </td>
                  <td><span className="fil-mono">{s.spool?.material}</span></td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        <SectionLabel>Swatches</SectionLabel>
        <div className="ll__row">
          <Swatch kind="solid" color="#B39DDB" size={20} />
          <Swatch kind="coextruded" color="#1B1B1B" color2="#C0A060" size={20} />
          <Swatch kind="gradient" color="#B39DDB" color2="#2E7D32" size={20} />
          <Swatch kind="empty" size={20} />
        </div>
      </div>
    </div>
  );
}

function HitList({ hits }: { hits: ClipHit[] }) {
  return (
    <ul className="ll__hits">
      {hits.slice(0, 30).map((h, i) => (
        <li key={i}>
          <code>{h.what}</code> · {h.axis === "x" ? "width" : "height"} short by {Math.ceil(h.by)} px
        </li>
      ))}
      {hits.length > 30 && <li>…and {hits.length - 30} more</li>}
    </ul>
  );
}
