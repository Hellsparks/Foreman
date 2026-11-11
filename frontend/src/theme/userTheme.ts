/**
 * User themes (Phase 6.1).
 *
 * A theme is a JSON file of token values:
 *
 *   {
 *     "name": "Compact rounded",
 *     "tokens": { "--fw-radius": "8px", "--fw-font-body": "\"My Sans\", sans-serif" },
 *     "fonts": [{ "family": "My Sans", "file": "my-sans.woff2" }]
 *   }
 *
 * - Only the keys below are accepted; anything else is rejected on import.
 * - Font files live in `<data_dir>/fonts/` and are served at `/theme/fonts/<file>`.
 * - The applied theme is kept in this browser (localStorage), like scheme/mode.
 *
 * Contrast and tabular-figure problems are warnings, never blocks.
 */
import { apiUrl } from "../api/conn";

// ── Token catalogue ──────────────────────────────────────────────────────────

export type TokenKind = "length" | "number" | "font" | "color";

export interface TokenSpec {
  kind: TokenKind;
  label: string;
}

const LAYOUT: Record<string, TokenSpec> = {
  "--fw-radius": { kind: "length", label: "Corner radius" },
  "--fw-border-width": { kind: "length", label: "Border width" },
  "--fw-control-h": { kind: "length", label: "Control height" },
  "--fw-control-h-md": { kind: "length", label: "Control height, list rows" },
  "--fw-control-h-sm": { kind: "length", label: "Control height, small" },
  "--fw-space-1": { kind: "length", label: "Space 1" },
  "--fw-space-2": { kind: "length", label: "Space 2" },
  "--fw-space-3": { kind: "length", label: "Space 3" },
  "--fw-space-4": { kind: "length", label: "Space 4" },
  "--fw-space-5": { kind: "length", label: "Space 5" },
  "--fw-space-6": { kind: "length", label: "Space 6" },
  "--fw-rail-w": { kind: "length", label: "Sidebar width" },
};

const TYPE: Record<string, TokenSpec> = {
  "--fw-font-body": { kind: "font", label: "Body font" },
  "--fw-font-mono": { kind: "font", label: "Mono font" },
  "--fw-font-size": { kind: "length", label: "UI font size" },
  "--fw-line-height": { kind: "number", label: "Line height" },
};

const COLOR_NAMES = [
  "background", "surface", "raised", "text", "muted", "border",
  "accent", "accent-hover", "on-accent", "button-hover", "focus",
  "success", "warning", "danger", "info",
  "success-bg", "warning-bg", "danger-bg", "info-bg",
] as const;

const COLORS: Record<string, TokenSpec> = Object.fromEntries(
  COLOR_NAMES.map((n) => [`--fw-${n}`, { kind: "color" as const, label: n }]),
);

export const TOKENS: Record<string, TokenSpec> = { ...LAYOUT, ...TYPE, ...COLORS };

const TOP_KEYS = new Set(["name", "description", "tokens", "fonts"]);
const FONT_KEYS = new Set(["family", "file", "weight", "style"]);

// ── Types ────────────────────────────────────────────────────────────────────

export interface ThemeFont {
  family: string;
  file: string;
  weight?: string;
  style?: string;
}

export interface UserTheme {
  name: string;
  description?: string;
  tokens: Record<string, string>;
  fonts?: ThemeFont[];
}

export type ParseResult = { ok: true; theme: UserTheme } | { ok: false; errors: string[] };

// ── Validation ───────────────────────────────────────────────────────────────

const LENGTH_RE = /^(0|\d{1,3}(\.\d{1,3})?(px|rem|em))$/;
const NUMBER_RE = /^\d(\.\d{1,3})?$/;
const COLOR_RE = /^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/;
/** Font stacks: names, quotes, commas, spaces, hyphens. No url(), no `;`. */
const FONT_RE = /^[A-Za-z0-9 ,"'_-]{1,200}$/;
const FONT_FILE_RE = /^[A-Za-z0-9_-][A-Za-z0-9._-]{0,120}\.woff2$/;
const FAMILY_RE = /^[A-Za-z0-9 _-]{1,60}$/;
const WEIGHT_RE = /^(normal|bold|\d{3}( \d{3})?)$/;
const STYLE_RE = /^(normal|italic|oblique)$/;

function checkValue(key: string, value: unknown): string | null {
  if (typeof value !== "string") return `${key}: value must be a string`;
  const spec = TOKENS[key];
  const v = value.trim();
  switch (spec.kind) {
    case "length":
      return LENGTH_RE.test(v) ? null : `${key}: "${value}" is not a length like 8px`;
    case "number":
      return NUMBER_RE.test(v) ? null : `${key}: "${value}" is not a number like 1.5`;
    case "color":
      return COLOR_RE.test(v) ? null : `${key}: "${value}" is not a hex colour like #1A1B18`;
    case "font":
      return FONT_RE.test(v) && !/(\.\.|^,|,$)/.test(v)
        ? null
        : `${key}: "${value}" is not a font list like "Inter", Arial, sans-serif`;
  }
}

/** Parse and validate a theme file. Unknown keys anywhere are rejected. */
export function parseTheme(text: string): ParseResult {
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch (e) {
    return { ok: false, errors: [`Not valid JSON: ${e instanceof Error ? e.message : String(e)}`] };
  }
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
    return { ok: false, errors: ["A theme file is a JSON object"] };
  }
  const obj = raw as Record<string, unknown>;
  const errors: string[] = [];

  for (const k of Object.keys(obj)) {
    if (!TOP_KEYS.has(k)) errors.push(`Unknown key "${k}" (allowed: name, description, tokens, fonts)`);
  }
  if (typeof obj.name !== "string" || !obj.name.trim()) errors.push(`"name" must be a non-empty string`);
  else if (obj.name.length > 80) errors.push(`"name" is longer than 80 characters`);
  if (obj.description !== undefined && typeof obj.description !== "string") {
    errors.push(`"description" must be a string`);
  }

  const tokens: Record<string, string> = {};
  if (typeof obj.tokens !== "object" || obj.tokens === null || Array.isArray(obj.tokens)) {
    errors.push(`"tokens" must be an object of token → value`);
  } else {
    for (const [k, v] of Object.entries(obj.tokens as Record<string, unknown>)) {
      if (!(k in TOKENS)) {
        errors.push(`Unknown token "${k}"`);
        continue;
      }
      const err = checkValue(k, v);
      if (err) errors.push(err);
      else tokens[k] = (v as string).trim();
    }
  }

  const fonts: ThemeFont[] = [];
  if (obj.fonts !== undefined) {
    if (!Array.isArray(obj.fonts)) {
      errors.push(`"fonts" must be a list`);
    } else {
      obj.fonts.forEach((f: unknown, i: number) => {
        if (typeof f !== "object" || f === null || Array.isArray(f)) {
          errors.push(`fonts[${i}] must be an object`);
          return;
        }
        const fo = f as Record<string, unknown>;
        for (const k of Object.keys(fo)) {
          if (!FONT_KEYS.has(k)) errors.push(`fonts[${i}]: unknown key "${k}" (allowed: family, file, weight, style)`);
        }
        const bad = (k: string, re: RegExp, req: boolean) =>
          (fo[k] === undefined && !req) || (typeof fo[k] === "string" && re.test(fo[k] as string))
            ? null
            : `fonts[${i}].${k} is missing or invalid`;
        for (const e of [
          bad("family", FAMILY_RE, true),
          bad("file", FONT_FILE_RE, true),
          bad("weight", WEIGHT_RE, false),
          bad("style", STYLE_RE, false),
        ]) {
          if (e) errors.push(e);
        }
        if (!errors.some((e) => e.startsWith(`fonts[${i}]`))) {
          fonts.push({
            family: fo.family as string,
            file: fo.file as string,
            ...(fo.weight ? { weight: fo.weight as string } : {}),
            ...(fo.style ? { style: fo.style as string } : {}),
          });
        }
      });
    }
  }

  if (errors.length) return { ok: false, errors };
  return {
    ok: true,
    theme: {
      name: (obj.name as string).trim(),
      ...(typeof obj.description === "string" ? { description: obj.description } : {}),
      tokens,
      ...(fonts.length ? { fonts } : {}),
    },
  };
}

// ── Contrast (WCAG 2.x relative luminance) ───────────────────────────────────

export function hexToRgb(hex: string): [number, number, number] | null {
  let h = hex.trim().replace(/^#/, "");
  if (h.length === 3) h = h.split("").map((c) => c + c).join("");
  if (h.length === 8) h = h.slice(0, 6);
  if (!/^[0-9a-fA-F]{6}$/.test(h)) return null;
  return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16)) as [number, number, number];
}

/** Accepts `#rrggbb` or the `rgb(r, g, b)` form getComputedStyle may return. */
export function parseColor(c: string): [number, number, number] | null {
  const m = c.match(/^rgba?\(\s*(\d+)[,\s]+(\d+)[,\s]+(\d+)/);
  if (m) return [Number(m[1]), Number(m[2]), Number(m[3])];
  return hexToRgb(c);
}

function luminance([r, g, b]: [number, number, number]): number {
  const f = (v: number) => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}

export function contrastRatio(a: string, b: string): number | null {
  const ca = parseColor(a);
  const cb = parseColor(b);
  if (!ca || !cb) return null;
  const [hi, lo] = [luminance(ca), luminance(cb)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** [foreground, background, minimum ratio, what it is] */
export const CONTRAST_PAIRS: [string, string, number, string][] = [
  ["--fw-text", "--fw-background", 4.5, "text on background"],
  ["--fw-text", "--fw-surface", 4.5, "text on surface"],
  ["--fw-text", "--fw-raised", 4.5, "text on panels"],
  ["--fw-muted", "--fw-background", 4.5, "muted text on background"],
  ["--fw-muted", "--fw-raised", 4.5, "muted text on panels"],
  ["--fw-on-accent", "--fw-accent-hover", 4.5, "primary button label"],
  ["--fw-success", "--fw-success-bg", 4.5, "success chip"],
  ["--fw-warning", "--fw-warning-bg", 4.5, "warning chip"],
  ["--fw-danger", "--fw-danger-bg", 4.5, "danger chip"],
  ["--fw-info", "--fw-info-bg", 4.5, "info chip"],
  ["--fw-muted", "--fw-background", 3, "control boundary"],
  ["--fw-focus", "--fw-background", 3, "focus outline"],
  ["--fw-accent-hover", "--fw-background", 3, "primary button fill"],
];

/**
 * Contrast warnings for the pairs a theme touches. `resolve` gives the value a
 * token has without the theme (the active scheme), for the side it leaves alone.
 */
export function contrastWarnings(
  tokens: Record<string, string>,
  resolve: (token: string) => string,
): string[] {
  const out: string[] = [];
  for (const [fg, bg, min, what] of CONTRAST_PAIRS) {
    if (!(fg in tokens) && !(bg in tokens)) continue;
    const r = contrastRatio(tokens[fg] ?? resolve(fg), tokens[bg] ?? resolve(bg));
    if (r !== null && r < min) {
      out.push(`Contrast ${r.toFixed(2)}:1 for ${what} (${fg} on ${bg}); needs ${min}:1`);
    }
  }
  return out;
}

// ── Fonts ────────────────────────────────────────────────────────────────────

export function fontUrl(file: string): string {
  return apiUrl(`/theme/fonts/${encodeURIComponent(file)}`);
}

const loadedFaces: FontFace[] = [];

/** Register the theme's @font-faces. Returns the files that failed to load. */
export async function registerFonts(fonts: ThemeFont[] | undefined): Promise<string[]> {
  for (const f of loadedFaces.splice(0)) document.fonts.delete(f);
  const failed: string[] = [];
  await Promise.all(
    (fonts ?? []).map(async (f) => {
      const face = new FontFace(f.family, `url("${fontUrl(f.file)}") format("woff2")`, {
        weight: f.weight ?? "100 900",
        style: f.style ?? "normal",
        display: "swap",
      });
      document.fonts.add(face);
      loadedFaces.push(face);
      try {
        await face.load();
      } catch {
        failed.push(f.file);
      }
    }),
  );
  return failed;
}

/**
 * True when digits in `family` have equal widths with tabular-nums on.
 * Temperatures and weights jitter as they update when this is false.
 */
export function hasTabularFigures(family: string): boolean {
  const mk = (text: string) => {
    const s = document.createElement("span");
    s.textContent = text;
    s.style.cssText =
      "position:absolute;visibility:hidden;white-space:pre;font-size:40px;font-variant-numeric:tabular-nums;";
    s.style.fontFamily = family;
    document.body.appendChild(s);
    const w = s.getBoundingClientRect().width;
    s.remove();
    return w;
  };
  const widths = ["0000000000", "1111111111", "4444444444", "7777777777"].map(mk);
  return Math.max(...widths) - Math.min(...widths) < 1;
}

// ── Apply / persist ──────────────────────────────────────────────────────────

const THEME_KEY = "fw-user-theme";
const CUSTOM_CSS_KEY = "fw-custom-css";
const STYLE_ID = "fw-user-theme";
const CUSTOM_LINK_ID = "fw-custom-css";

function store(key: string, value: string | null) {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    /* storage unavailable: the theme still applies for this session */
  }
}

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

/**
 * CSS for a theme. The selector outranks both tokens.css's scheme rules
 * (`:root[data-fw-theme][data-fw-mode]`) and theme.css's `:root`.
 */
export function themeCss(theme: UserTheme): string {
  const lines = Object.entries(theme.tokens)
    .filter(([k]) => k in TOKENS)
    .map(([k, v]) => `  ${k}: ${v};`);
  return `html:root[data-fw-theme][data-fw-mode] {\n${lines.join("\n")}\n}\n`;
}

/** Apply (or clear, with null) a theme on the page, keeping custom.css last. */
export function applyTheme(theme: UserTheme | null): void {
  document.getElementById(STYLE_ID)?.remove();
  if (theme) {
    const el = document.createElement("style");
    el.id = STYLE_ID;
    el.textContent = themeCss(theme);
    document.head.appendChild(el);
  }
  const link = document.getElementById(CUSTOM_LINK_ID);
  if (link) document.head.appendChild(link); // move after the theme
}

export function savedTheme(): UserTheme | null {
  const raw = read(THEME_KEY);
  if (!raw) return null;
  const r = parseTheme(raw);
  return r.ok ? r.theme : null;
}

export function saveTheme(theme: UserTheme | null): void {
  store(THEME_KEY, theme ? JSON.stringify(theme) : null);
}

export function customCssEnabled(): boolean {
  return read(CUSTOM_CSS_KEY) === "1";
}

export function setCustomCss(on: boolean): void {
  store(CUSTOM_CSS_KEY, on ? "1" : null);
  applyCustomCss(on);
}

/** Add/remove `<link href="/theme/custom.css">`, always after the theme. */
export function applyCustomCss(on: boolean): void {
  document.getElementById(CUSTOM_LINK_ID)?.remove();
  if (!on) return;
  const link = document.createElement("link");
  link.id = CUSTOM_LINK_ID;
  link.rel = "stylesheet";
  link.href = apiUrl("/theme/custom.css");
  document.head.appendChild(link);
}

/** Startup: apply the saved theme, its fonts and custom.css. */
export function initUserTheme(): void {
  const theme = savedTheme();
  applyTheme(theme);
  if (theme?.fonts) void registerFonts(theme.fonts);
  applyCustomCss(customCssEnabled());
}

/** The current value of every themeable token, as a theme file to edit. */
export function exportCurrent(name: string): UserTheme {
  const cs = getComputedStyle(document.documentElement);
  const saved = savedTheme();
  const tokens: Record<string, string> = {};
  for (const k of Object.keys(TOKENS)) {
    let v = cs.getPropertyValue(k).trim();
    if (TOKENS[k].kind === "color") {
      const rgb = parseColor(v);
      if (rgb) v = "#" + rgb.map((n) => n.toString(16).padStart(2, "0")).join("").toUpperCase();
    }
    if (v) tokens[k] = v;
  }
  return {
    name: saved?.name ?? name,
    tokens,
    ...(saved?.fonts ? { fonts: saved.fonts } : {}),
  };
}

/** Value of a token without the user theme (for contrast against the scheme). */
export function schemeValue(token: string): string {
  const el = document.getElementById(STYLE_ID) as HTMLStyleElement | null;
  const prev = el?.disabled;
  if (el) el.disabled = true;
  const v = getComputedStyle(document.documentElement).getPropertyValue(token).trim();
  if (el) el.disabled = prev ?? false;
  return v;
}
