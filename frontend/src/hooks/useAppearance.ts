import { useCallback, useState } from "react";

export const THEMES = ["fieldwork", "vector", "foundry", "olive"] as const;
export const MODES = ["light", "dark"] as const;

export type Theme = (typeof THEMES)[number];
export type Mode = (typeof MODES)[number];

const DEFAULT_THEME: Theme = "fieldwork";
const DEFAULT_MODE: Mode = "dark";

function readTheme(): Theme {
  const v = localStorage.getItem("fw-theme");
  return THEMES.includes(v as Theme) ? (v as Theme) : DEFAULT_THEME;
}

function readMode(): Mode {
  const v = localStorage.getItem("fw-mode");
  return MODES.includes(v as Mode) ? (v as Mode) : DEFAULT_MODE;
}

function applyAppearance(theme: Theme, mode: Mode) {
  const root = document.documentElement;
  root.setAttribute("data-fw-theme", theme);
  root.setAttribute("data-fw-mode", mode);
}

/** Manages theme scheme + light/dark mode, persisted to localStorage. */
export function useAppearance() {
  const [theme, setThemeState] = useState<Theme>(readTheme);
  const [mode, setModeState] = useState<Mode>(readMode);

  const setTheme = useCallback((next: Theme) => {
    setThemeState(next);
    applyAppearance(next, readMode());
    localStorage.setItem("fw-theme", next);
  }, []);

  const setMode = useCallback((next: Mode) => {
    setModeState(next);
    applyAppearance(readTheme(), next);
    localStorage.setItem("fw-mode", next);
  }, []);

  return { theme, mode, setTheme, setMode };
}
