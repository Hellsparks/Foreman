import type { ColourKind } from "../api/types/ColourKind";
import type { SpoolFacts } from "../api/types/SpoolFacts";

/**
 * Filament swatch (docs/ui-ref README rule 8): single = filled circle,
 * coextruded = split circle, gradient = bar, empty = dashed.
 * Spool colours are data, so they are inline styles; everything else is tokens.
 */
export function ColourSwatch({
  colours,
  kind,
  size = 28,
  wide = false,
}: {
  /** `RRGGBB` or `#RRGGBB`. Empty → dashed empty swatch. */
  colours: string[];
  kind: ColourKind;
  size?: number;
  /** Gradient bar fills the width (form preview). */
  wide?: boolean;
}) {
  const cols = colours.map((h) => `#${h.replace(/^#/, "")}`);
  if (cols.length > 1 && kind === "coaxial") {
    const n = cols.length;
    const stops = cols
      .map((c, i) => `${c} ${Math.round((i / n) * 100)}% ${Math.round(((i + 1) / n) * 100)}%`)
      .join(", ");
    return (
      <span
        className="spool-swatch spool-swatch--circle"
        style={{ width: size, height: size, background: `conic-gradient(${stops})` }}
        aria-hidden="true"
      />
    );
  }
  if (cols.length > 1) {
    return (
      <span
        className="spool-swatch spool-swatch--bar"
        style={{
          width: wide ? "100%" : size * 1.6,
          height: Math.round(size * 0.6),
          background: `linear-gradient(90deg, ${cols.join(", ")})`,
        }}
        aria-hidden="true"
      />
    );
  }
  if (!cols.length) {
    return (
      <span
        className="spool-swatch spool-swatch--circle spool-swatch--empty"
        style={{ width: size, height: size }}
        aria-hidden="true"
      />
    );
  }
  return (
    <span
      className="spool-swatch spool-swatch--circle"
      style={{ width: size, height: size, background: cols[0] }}
      aria-hidden="true"
    />
  );
}

export default function SpoolSwatch({
  spool,
  hex,
  size = 28,
}: {
  spool?: SpoolFacts | null;
  /** Bare colour (e.g. a Bambu tray) when there is no spool. */
  hex?: string | null;
  size?: number;
}) {
  const multi = spool?.multi_color_hexes ?? [];
  if (multi.length > 1) {
    const kind: ColourKind = spool?.multi_color_direction === "coaxial" ? "coaxial" : "longitudinal";
    return <ColourSwatch colours={multi} kind={kind} size={size} />;
  }
  const single = spool?.color_hex ?? hex ?? null;
  return <ColourSwatch colours={single ? [single] : []} kind="single" size={size} />;
}
