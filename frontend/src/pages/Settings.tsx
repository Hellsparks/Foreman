import { useAppearance, THEMES, MODES, type Theme, type Mode } from "../hooks/useAppearance";

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

  return (
    <div>
      <div className="page-header">
        <p className="page-header__eyebrow">Settings</p>
        <h1 className="page-header__title">Settings</h1>
      </div>

      {/* Appearance ─────────────────────────────────────────────────────── */}
      <section aria-labelledby="appearance-heading" style={{ maxWidth: 640 }}>
        <h2
          id="appearance-heading"
          style={{ font: "500 18px/1.3 var(--fw-sans)", margin: "0 0 16px", letterSpacing: "-0.02em" }}
        >
          Appearance
        </h2>

        {/* Theme selector */}
        <fieldset style={{ border: 0, padding: 0, margin: "0 0 24px" }}>
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
          <div style={{ display: "flex", gap: 8 }}>
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
      </section>

      <p className="page-placeholder" style={{ marginTop: 32 }}>
        Server config, API keys, backup, notifications — coming in phase 7.
      </p>
    </div>
  );
}
