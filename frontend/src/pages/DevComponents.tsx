/**
 * /dev/components — dev-only component gallery.
 * Hidden from the navigation rail; reachable by URL only.
 */
import { useState } from "react";
import {
  Button,
  IconButton,
  BackButton,
  Panel,
  StatusChip,
  Swatch,
  SectionLabel,
  ProgressBar,
  ToastStack,
  useToasts,
} from "../ui";
import type { Tone } from "../ui";

const STATUS_CHIPS: { tone: Tone; symbol: string; word: string }[] = [
  { tone: "ok",   symbol: "✓", word: "Finished" },
  { tone: "info", symbol: "▶", word: "Printing" },
  { tone: "warn", symbol: "‖", word: "Paused"   },
  { tone: "bad",  symbol: "✖", word: "Error"    },
  { tone: "off",  symbol: "○", word: "Offline"  },
];

function Row({ children, style }: { children: React.ReactNode; style?: React.CSSProperties }) {
  return (
    <div style={{ display: "flex", flexWrap: "wrap", gap: "var(--fw-space-2)", alignItems: "center", ...style }}>
      {children}
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section style={{ marginBottom: 40 }}>
      <SectionLabel as="h2" style={{ marginBottom: "var(--fw-space-3)" } as React.CSSProperties}>
        {title}
      </SectionLabel>
      {children}
    </section>
  );
}

export default function DevComponents() {
  const [progressVal, setProgressVal] = useState(60);
  const { toasts, addToast, removeToast } = useToasts();

  return (
    <div style={{ padding: "var(--fw-space-6) 32px 64px", maxWidth: 900 }}>
      <div className="page-header">
        <p className="page-header__eyebrow">Dev</p>
        <h1 className="page-header__title">Component Gallery</h1>
      </div>

      {/* Button ─────────────────────────────────────────────────────────── */}
      <Section title="Button">
        <Row>
          <Button variant="primary">Primary</Button>
          <Button variant="secondary">Secondary</Button>
          <Button variant="small">Small</Button>
          <Button variant="danger">Danger</Button>
        </Row>
        <Row style={{ marginTop: "var(--fw-space-2)" } as React.CSSProperties}>
          <Button variant="primary" fixedWidth={148}>Fixed 148 px</Button>
          <Button variant="secondary" fixedWidth={148}>Fixed 148 px</Button>
          <Button variant="primary" disabled>Disabled</Button>
          <Button variant="secondary" disabled>Disabled</Button>
        </Row>
      </Section>

      {/* IconButton ──────────────────────────────────────────────────────── */}
      <Section title="IconButton">
        <Row>
          <IconButton aria-label="Menu (36 px)" size={36}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
              <circle cx="5"  cy="12" r="2" />
              <circle cx="12" cy="12" r="2" />
              <circle cx="19" cy="12" r="2" />
            </svg>
          </IconButton>
          <IconButton aria-label="Close (40 px)" size={40}>✕</IconButton>
          <IconButton aria-label="Settings (44 px)" size={44}>
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
              <path d="M12 9a3 3 0 100 6 3 3 0 000-6zM4 12h2M18 12h2M12 4v2M12 18v2" />
            </svg>
          </IconButton>
        </Row>
      </Section>

      {/* BackButton ──────────────────────────────────────────────────────── */}
      <Section title="BackButton">
        <Row>
          <BackButton label="Back to Fleet" onClick={() => {}} />
          <BackButton label="Back" onClick={() => {}} />
        </Row>
      </Section>

      {/* Panel ───────────────────────────────────────────────────────────── */}
      <Section title="Panel">
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--fw-space-4)" }}>
          <Panel>
            Plain panel body, no header.
          </Panel>
          <Panel header="Attention · 3">
            Panel with mono header only.
          </Panel>
          <Panel
            header="Current job"
            actions={
              <>
                <Button variant="small">View all</Button>
                <Button variant="small">Collapse</Button>
              </>
            }
          >
            Panel with header and right-side actions.
          </Panel>
        </div>
      </Section>

      {/* SectionLabel ────────────────────────────────────────────────────── */}
      <Section title="SectionLabel">
        <Row>
          <SectionLabel>Loaded filament</SectionLabel>
          <SectionLabel>Temperatures</SectionLabel>
          <SectionLabel>Attention · 5</SectionLabel>
        </Row>
      </Section>

      {/* StatusChip ──────────────────────────────────────────────────────── */}
      <Section title="StatusChip">
        <Row>
          {STATUS_CHIPS.map(({ tone, symbol, word }) => (
            <StatusChip key={tone} tone={tone} symbol={symbol} word={word} />
          ))}
        </Row>
      </Section>

      {/* Swatch ──────────────────────────────────────────────────────────── */}
      <Section title="Swatch">
        <Row>
          <Swatch kind="solid"      color="#1f8a8a" size={16} />
          <Swatch kind="solid"      color="#f4f4ef" size={16} />
          <Swatch kind="solid"      color="#1d1d22" size={16} />
          <Swatch kind="coextruded" color="#1f8a8a" color2="#f4f4ef" size={20} />
          <Swatch kind="gradient"   color="#1f8a8a" color2="#f4f4ef" size={16} />
          <Swatch kind="empty"      size={16} />
          <span style={{ color: "var(--fw-muted)", fontSize: 12, fontFamily: "var(--fw-mono)" }}>
            solid · solid · solid · coextruded · gradient · empty
          </span>
        </Row>
      </Section>

      {/* ProgressBar ─────────────────────────────────────────────────────── */}
      <Section title="ProgressBar">
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--fw-space-3)" }}>
          <ProgressBar value={progressVal} label="Adjustable progress" />
          <ProgressBar value={100} label="Complete" color="var(--fw-success)" height={8} />
          <ProgressBar value={0}   label="Zero" height={4} />
          <div style={{ display: "flex", alignItems: "center", gap: "var(--fw-space-3)", marginTop: "var(--fw-space-1)" }}>
            <input
              type="range"
              min={0}
              max={100}
              value={progressVal}
              onChange={(e) => setProgressVal(Number(e.target.value))}
              style={{ flex: 1, maxWidth: 300 }}
              aria-label="Adjust progress demo value"
            />
            <span style={{ fontFamily: "var(--fw-mono)", fontSize: 12, color: "var(--fw-muted)", minWidth: 40 }}>
              {progressVal}%
            </span>
          </div>
        </div>
      </Section>

      {/* ToastStack ──────────────────────────────────────────────────────── */}
      <Section title="ToastStack">
        <Row>
          <Button
            variant="secondary"
            onClick={() => addToast("Print job sent to Bambu X1C.")}
          >
            Add toast
          </Button>
          <Button
            variant="secondary"
            onClick={() =>
              addToast("Plate cleared on Bambu X1C.", () =>
                addToast("Undo applied.")
              )
            }
          >
            Add toast with Undo
          </Button>
        </Row>
        <p style={{ marginTop: "var(--fw-space-2)", fontSize: 13, color: "var(--fw-muted)", fontFamily: "var(--fw-mono)" }}>
          Toasts appear fixed bottom-right.
        </p>
      </Section>

      <ToastStack toasts={toasts} onDismiss={removeToast} />
    </div>
  );
}
