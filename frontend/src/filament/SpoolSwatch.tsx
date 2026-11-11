import type { SpoolFacts } from "../api/types/SpoolFacts";

/**
 * Filament swatch (docs/ui-ref README rule 8): single = filled circle,
 * coextruded = split circle, gradient = bar, empty = dashed.
 * Spool colours are data, so they are inline styles; everything else is tokens.
 */
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
  const single = spool?.color_hex ?? hex ?? null;
  if (multi.length > 1) {
    const cols = multi.map((h) => `#${h}`);
    if (spool?.multi_color_direction === "coaxial") {
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
    return (
      <span
        className="spool-swatch spool-swatch--bar"
        style={{ width: size * 1.6, height: Math.round(size * 0.6), background: `linear-gradient(90deg, ${cols.join(", ")})` }}
        aria-hidden="true"
      />
    );
  }
  if (!single) {
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
      style={{ width: size, height: size, background: `#${single.replace(/^#/, "")}` }}
      aria-hidden="true"
    />
  );
}
