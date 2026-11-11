import { describe, expect, it } from "vitest";
import sampleTheme from "../../public/themes/compact-rounded.json?raw";
import { contrastRatio, contrastWarnings, parseTheme, themeCss, TOKENS } from "./userTheme";

const ok = (text: string) => {
  const r = parseTheme(text);
  if (!r.ok) throw new Error(r.errors.join("; "));
  return r.theme;
};
const errs = (v: unknown) => {
  const r = parseTheme(JSON.stringify(v));
  return r.ok ? [] : r.errors;
};

describe("parseTheme", () => {
  it("accepts a minimal theme", () => {
    const t = ok(JSON.stringify({ name: "x", tokens: { "--fw-radius": "8px" } }));
    expect(t.tokens).toEqual({ "--fw-radius": "8px" });
  });

  it("rejects unknown keys at every level", () => {
    expect(errs({ name: "x", tokens: {}, colours: {} })).toEqual([expect.stringContaining('Unknown key "colours"')]);
    expect(errs({ name: "x", tokens: { "--fw-sans": "Arial" } })).toEqual([expect.stringContaining('Unknown token "--fw-sans"')]);
    expect(errs({ name: "x", tokens: {}, fonts: [{ family: "A", file: "a.woff2", src: "x" }] })).toEqual([
      expect.stringContaining('unknown key "src"'),
    ]);
  });

  it("rejects values that could break out of the declaration", () => {
    expect(errs({ name: "x", tokens: { "--fw-radius": "2px; } body { display:none" } })).toHaveLength(1);
    expect(errs({ name: "x", tokens: { "--fw-font-body": "Arial; } *{color:red" } })).toHaveLength(1);
    expect(errs({ name: "x", tokens: { "--fw-font-body": "url(x)" } })).toHaveLength(1);
    expect(errs({ name: "x", tokens: { "--fw-accent": "red" } })).toHaveLength(1);
    expect(errs({ name: "x", tokens: {}, fonts: [{ family: "A", file: "../secret.key" }] })).toHaveLength(1);
  });

  it("rejects non-JSON and non-objects", () => {
    expect(parseTheme("{").ok).toBe(false);
    expect(parseTheme("[]").ok).toBe(false);
    expect(errs({ tokens: {} })).toEqual([expect.stringContaining('"name"')]);
  });

  it("accepts the shipped test theme", () => {
    const t = ok(sampleTheme);
    for (const k of ["--fw-radius", "--fw-space-2", "--fw-control-h", "--fw-font-body", "--fw-font-mono"]) {
      expect(t.tokens[k]).toBeTruthy();
    }
    expect(t.fonts?.length).toBe(2);
  });
});

describe("themeCss", () => {
  it("emits only known tokens under a selector that beats the scheme rules", () => {
    const css = themeCss({ name: "x", tokens: { "--fw-radius": "8px", "--evil": "1" } });
    expect(css).toContain("html:root[data-fw-theme][data-fw-mode]");
    expect(css).toContain("--fw-radius: 8px;");
    expect(css).not.toContain("--evil");
  });
});

describe("contrast", () => {
  it("matches known WCAG ratios", () => {
    expect(contrastRatio("#000000", "#FFFFFF")).toBeCloseTo(21, 1);
    expect(contrastRatio("#777777", "#FFFFFF")).toBeCloseTo(4.48, 2);
    expect(contrastRatio("rgb(255, 255, 255)", "#fff")).toBeCloseTo(1, 5);
  });

  it("warns (4.5 text, 3 UI) only for pairs the theme touches", () => {
    const resolve = (t: string) => (t === "--fw-background" ? "#FFFFFF" : "#000000");
    expect(contrastWarnings({ "--fw-radius": "4px" }, resolve)).toEqual([]);
    const w = contrastWarnings({ "--fw-text": "#999999" }, resolve);
    expect(w.some((x) => x.includes("text on background") && x.includes("4.5:1"))).toBe(true);
    const f = contrastWarnings({ "--fw-focus": "#CCCCCC" }, resolve);
    expect(f.some((x) => x.includes("focus outline") && x.includes("3:1"))).toBe(true);
  });

  it("covers every colour token in the catalogue with a valid kind", () => {
    expect(Object.values(TOKENS).every((s) => ["length", "number", "font", "color"].includes(s.kind))).toBe(true);
  });
});
