/**
 * Settings → Appearance: theme file import/export, custom.css, longest labels.
 */
import { useRef, useState } from "react";
import { Button, SectionLabel } from "../ui";
import { saveBlob } from "../api/client";
import {
  applyTheme,
  contrastWarnings,
  customCssEnabled,
  exportCurrent,
  hasTabularFigures,
  parseTheme,
  registerFonts,
  savedTheme,
  saveTheme,
  schemeValue,
  setCustomCss,
  type UserTheme,
} from "./userTheme";
import { announceThemeApplied } from "./events";
import LongestLabels from "./LongestLabels";
import "./theme-settings.css";

/** Contrast, tabular-figure and missing-font warnings for an applied theme. */
export async function themeWarnings(theme: UserTheme): Promise<string[]> {
  const failed = await registerFonts(theme.fonts);
  const warnings = failed.map(
    (f) => `Font file "${f}" did not load. Put it in the data folder under fonts/ (falls back to the next font in the list).`,
  );
  warnings.push(...contrastWarnings(theme.tokens, schemeValue));
  const mono = theme.tokens["--fw-font-mono"];
  if (mono) {
    await document.fonts.ready;
    if (!hasTabularFigures(mono)) {
      warnings.push(
        "The mono font has no tabular figures: temperatures and weights will jitter as they update.",
      );
    }
  }
  return warnings;
}

export default function ThemeSettings() {
  const [theme, setTheme] = useState<UserTheme | null>(savedTheme);
  const [errors, setErrors] = useState<string[]>([]);
  const [warnings, setWarnings] = useState<string[]>([]);
  const [customCss, setCustomCssState] = useState(customCssEnabled);
  const [showLabels, setShowLabels] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  const importFile = async (file: File) => {
    setErrors([]);
    setWarnings([]);
    const r = parseTheme(await file.text());
    if (!r.ok) {
      setErrors(r.errors);
      return;
    }
    applyTheme(r.theme);
    saveTheme(r.theme);
    setTheme(r.theme);
    announceThemeApplied();
    setWarnings(await themeWarnings(r.theme));
    announceThemeApplied(); // again, now that fonts are in
  };

  const remove = () => {
    applyTheme(null);
    saveTheme(null);
    void registerFonts([]);
    setTheme(null);
    setErrors([]);
    setWarnings([]);
    announceThemeApplied();
  };

  const exportTheme = () => {
    const t = exportCurrent("My theme");
    const slug = t.name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "theme";
    saveBlob(new Blob([JSON.stringify(t, null, 2) + "\n"], { type: "application/json" }), `${slug}.json`);
  };

  return (
    <div className="ths">
      <fieldset className="ths__group">
        <legend className="ths__legend">Theme file</legend>
        <p className="ths__current" data-testid="theme-current">
          {theme ? (
            <>
              Using <b>{theme.name}</b>
              {theme.description ? ` · ${theme.description}` : ""}
            </>
          ) : (
            "Built-in tokens (no theme file)"
          )}
        </p>
        <div className="ths__actions">
          <Button onClick={() => fileRef.current?.click()}>Import theme…</Button>
          <Button onClick={exportTheme}>Export current</Button>
          {theme && <Button onClick={remove}>Remove theme</Button>}
          <input
            ref={fileRef}
            type="file"
            accept=".json,application/json"
            hidden
            data-testid="theme-file"
            onChange={(e) => {
              const f = e.target.files?.[0];
              e.target.value = "";
              if (f) void importFile(f);
            }}
          />
        </div>
        <p className="ths__help">
          A theme is a JSON file of token values (radius, spacing, control heights, body and mono fonts,
          colours). Export gives you the current values to edit. Fonts: put <code>.woff2</code> files in the
          server's data folder under <code>fonts/</code> and list them in the theme's <code>fonts</code>.
        </p>
        {errors.length > 0 && (
          <div className="ths__msg ths__msg--bad" role="alert">
            <b>Not imported:</b>
            <ul>{errors.map((e) => <li key={e}>{e}</li>)}</ul>
          </div>
        )}
        {warnings.length > 0 && (
          <div className="ths__msg ths__msg--warn" role="status">
            <b>Imported, with warnings:</b>
            <ul>{warnings.map((w) => <li key={w}>{w}</li>)}</ul>
          </div>
        )}
      </fieldset>

      <fieldset className="ths__group">
        <legend className="ths__legend">Custom CSS</legend>
        <label className="ths__check">
          <input
            type="checkbox"
            checked={customCss}
            onChange={(e) => {
              setCustomCss(e.target.checked);
              setCustomCssState(e.target.checked);
              announceThemeApplied();
            }}
          />
          Load <code>custom.css</code> from the server's data folder
        </label>
        <p className="ths__help">
          Unsupported: it loads after the theme and can break any screen. Updates may change the class names
          it relies on. Turn it off here if something looks wrong.
        </p>
      </fieldset>

      <fieldset className="ths__group">
        <legend className="ths__legend">Longest labels</legend>
        <p className="ths__help">
          Renders the longest printer and filament names, the widest status chips and fixed-width buttons in
          the current theme, and flags anything that clips.
        </p>
        <div className="ths__actions">
          <Button aria-expanded={showLabels} onClick={() => setShowLabels((s) => !s)}>
            {showLabels ? "Hide check" : "Check longest labels"}
          </Button>
        </div>
        {showLabels && (
          <>
            <SectionLabel>Preview</SectionLabel>
            <LongestLabels />
          </>
        )}
      </fieldset>
    </div>
  );
}
