export type SwatchKind = "solid" | "coextruded" | "gradient" | "empty";

interface SwatchProps {
  kind?: SwatchKind;
  /** Hex color for solid fill, or first half of coextruded, or gradient start. */
  color?: string;
  /** Second color for coextruded split circle or gradient end. */
  color2?: string;
  /** Display size in px (applies to both width and height). Default 16. */
  size?: number;
  className?: string;
}

/**
 * Filament colour swatch.
 * - solid: filled square/circle in one colour
 * - coextruded: split circle (two colours, coaxial)
 * - gradient: horizontal gradient bar (longitudinal)
 * - empty: dashed-border transparent square
 */
export default function Swatch({
  kind = "solid",
  color,
  color2,
  size = 16,
  className,
}: SwatchProps) {
  const base = `ui-swatch ui-swatch--${kind}${className ? ` ${className}` : ""}`;

  if (kind === "coextruded") {
    return (
      <span
        className={base}
        style={{ width: size, height: size }}
        aria-hidden="true"
      >
        <span className="ui-swatch__half" style={{ background: color ?? "transparent" }} />
        <span className="ui-swatch__half" style={{ background: color2 ?? "transparent" }} />
      </span>
    );
  }

  if (kind === "gradient") {
    return (
      <span
        className={base}
        style={{
          width: size * 3,
          height: size,
          background: `linear-gradient(90deg, ${color ?? "transparent"}, ${color2 ?? "transparent"})`,
        }}
        aria-hidden="true"
      />
    );
  }

  // solid + empty both use a plain box; color applied via inline style for solid
  return (
    <span
      className={base}
      style={{ width: size, height: size, background: kind === "solid" ? color : undefined }}
      aria-hidden="true"
    />
  );
}
