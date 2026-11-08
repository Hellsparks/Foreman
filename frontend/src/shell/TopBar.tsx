import type { ConnectionState } from "../hooks/useEventStream";
import { THEMES, type Mode, type Theme } from "../hooks/useAppearance";

// Accent colors for the 4×2 theme swatches (purely decorative preview chips).
// These are the accent-hover values from tokens.css — not used for styling,
// only for the visual swatch squares.
const THEME_ACCENTS: Record<Theme, Record<Mode, string>> = {
  fieldwork: { light: "#BF3D1C", dark: "#F57951" },
  vector: { light: "#455900", dark: "#D5FA39" },
  foundry: { light: "#3D3D3D", dark: "#D5D5D5" },
  olive: { light: "#5D622D", dark: "#C2C78D" },
};

const CONN_LABEL: Record<ConnectionState, string> = {
  connecting: "Connecting",
  connected: "Live",
  disconnected: "Offline",
};

interface TopBarProps {
  connectionState: ConnectionState;
  theme: Theme;
  mode: Mode;
  onThemeChange: (t: Theme) => void;
  onModeChange: (m: Mode) => void;
  attentionCount: number;
}

export default function TopBar({
  connectionState,
  theme,
  mode,
  onThemeChange,
  onModeChange,
  attentionCount,
}: TopBarProps) {
  return (
    <header className="topbar" role="banner">
      {/* Command palette trigger */}
      <button
        type="button"
        className="topbar__search"
        aria-label="Open command palette"
        aria-keyshortcuts="Control+k"
        onClick={() => {
          /* command palette — session 7 */
        }}
      >
        <svg
          width="14"
          height="14"
          viewBox="0 0 16 16"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          aria-hidden="true"
        >
          <circle cx="7" cy="7" r="4.5" />
          <line x1="10.5" y1="10.5" x2="14" y2="14" />
        </svg>
        <span>Search or run a command</span>
        <kbd className="topbar__kbd">Ctrl K</kbd>
      </button>

      {/* Spacer */}
      <div style={{ flex: 1 }} />

      {/* Scan button */}
      <button type="button" className="topbar__btn" aria-label="Scan QR or NFC tag">
        <svg
          width="16"
          height="16"
          viewBox="0 0 16 16"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          aria-hidden="true"
        >
          <rect x="1" y="1" width="5" height="5" />
          <rect x="10" y="1" width="5" height="5" />
          <rect x="1" y="10" width="5" height="5" />
          <rect x="10" y="10" width="2" height="2" />
          <rect x="13" y="10" width="2" height="2" />
          <rect x="10" y="13" width="2" height="2" />
          <rect x="13" y="13" width="2" height="2" />
        </svg>
        <span>Scan</span>
      </button>

      {/* Send (upload) button */}
      <button type="button" className="topbar__btn" aria-label="Upload a file to send to a printer">
        <svg
          width="16"
          height="16"
          viewBox="0 0 16 16"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          aria-hidden="true"
        >
          <path d="M8 10V3M5 6l3-3 3 3" />
          <path d="M2 12v1a1 1 0 001 1h10a1 1 0 001-1v-1" />
        </svg>
        <span>Send</span>
      </button>

      {/* Attention count */}
      <button
        type="button"
        className="topbar__btn"
        aria-label={`Attention — ${attentionCount} item${attentionCount === 1 ? "" : "s"} need action`}
      >
        <svg
          width="16"
          height="16"
          viewBox="0 0 16 16"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          aria-hidden="true"
        >
          <path d="M8 1.5L1.5 12.5h13L8 1.5z" />
          <line x1="8" y1="6" x2="8" y2="9" />
          <circle cx="8" cy="11" r="0.5" fill="currentColor" />
        </svg>
        {attentionCount > 0 && (
          <span className="topbar__attention-count" aria-hidden="true">
            {attentionCount}
          </span>
        )}
      </button>

      {/* Appearance: 4 theme swatches then mode toggle */}
      <div
        className="topbar__appearance"
        role="group"
        aria-label="Appearance"
      >
        {THEMES.map((t) => (
          <button
            key={t}
            type="button"
            className="topbar__swatch"
            style={{ background: THEME_ACCENTS[t][mode] }}
            aria-label={`${t} theme`}
            aria-pressed={theme === t}
            onClick={() => onThemeChange(t)}
          />
        ))}
        <button
          type="button"
          className="topbar__btn"
          aria-label={mode === "dark" ? "Switch to light mode" : "Switch to dark mode"}
          aria-pressed={mode === "dark"}
          onClick={() => onModeChange(mode === "dark" ? "light" : "dark")}
          style={{ minWidth: 44 }}
        >
          {mode === "dark" ? (
            /* sun icon */
            <svg
              width="16"
              height="16"
              viewBox="0 0 16 16"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.5"
              aria-hidden="true"
            >
              <circle cx="8" cy="8" r="3" />
              <line x1="8" y1="1" x2="8" y2="2.5" />
              <line x1="8" y1="13.5" x2="8" y2="15" />
              <line x1="1" y1="8" x2="2.5" y2="8" />
              <line x1="13.5" y1="8" x2="15" y2="8" />
              <line x1="3.1" y1="3.1" x2="4.1" y2="4.1" />
              <line x1="11.9" y1="11.9" x2="12.9" y2="12.9" />
              <line x1="3.1" y1="12.9" x2="4.1" y2="11.9" />
              <line x1="11.9" y1="4.1" x2="12.9" y2="3.1" />
            </svg>
          ) : (
            /* moon icon */
            <svg
              width="16"
              height="16"
              viewBox="0 0 16 16"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.5"
              aria-hidden="true"
            >
              <path d="M12 10A6 6 0 016 4a6 6 0 100 8 6 6 0 006-2z" />
            </svg>
          )}
        </button>
      </div>

      {/* Connection state */}
      <div
        className={`topbar__conn topbar__conn--${connectionState}`}
        aria-live="polite"
        aria-label={`Server connection: ${CONN_LABEL[connectionState]}`}
      >
        <span className="topbar__conn-dot" />
        <span className="topbar__conn-label">{CONN_LABEL[connectionState]}</span>
      </div>
    </header>
  );
}
