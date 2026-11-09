export type Tone = "ok" | "info" | "warn" | "bad" | "off";

interface StatusChipProps {
  tone: Tone;
  /** Unicode symbol, e.g. "▶", "✓", "‖", "✖", "○", "●" */
  symbol: string;
  /** Status word, e.g. "Printing", "Finished" */
  word: string;
}

export default function StatusChip({ tone, symbol, word }: StatusChipProps) {
  return (
    <span className={`ui-status-chip ui-status-chip--${tone}`}>
      {symbol} {word}
    </span>
  );
}
