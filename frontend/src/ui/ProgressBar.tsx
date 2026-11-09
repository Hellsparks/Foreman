interface ProgressBarProps {
  /** 0 – 100 */
  value: number;
  /** Override the fill colour with any CSS color string using --fw-* vars. */
  color?: string;
  /** Bar height in px. Default 6. */
  height?: number;
  /** Accessible label for screen readers. */
  label?: string;
}

export default function ProgressBar({
  value,
  color,
  height = 6,
  label = "Progress",
}: ProgressBarProps) {
  const clamped = Math.max(0, Math.min(100, value));
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuenow={clamped}
      aria-valuemin={0}
      aria-valuemax={100}
      className="ui-progress"
      style={{ height }}
    >
      <div
        className="ui-progress__fill"
        style={{
          width: `${clamped}%`,
          ...(color ? { background: color } : {}),
        }}
      />
    </div>
  );
}
