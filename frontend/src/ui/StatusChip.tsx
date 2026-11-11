export type Tone = "ok" | "info" | "warn" | "bad" | "off";

interface StatusChipProps {
  tone: Tone;
  /** Unicode symbol, e.g. "▶", "✓", "‖", "✖", "○", "●" */
  symbol: string;
  /** Status word, e.g. "Printing", "Finished" */
  word: string;
  /** Phone form, e.g. "63%" for "Printing 63%". The full word stays in the
   *  accessible name; the short form only replaces it visually below 760 px. */
  short?: string;
}

export default function StatusChip({ tone, symbol, word, short }: StatusChipProps) {
  if (short == null) {
    return (
      <span className={`ui-status-chip ui-status-chip--${tone}`}>
        {symbol} {word}
      </span>
    );
  }
  return (
    <span className={`ui-status-chip ui-status-chip--${tone}`}>
      <span className="ui-status-chip__long">
        {symbol} {word}
      </span>
      <span className="ui-status-chip__short">
        <span aria-hidden="true">
          {symbol} {short}
        </span>
        <span className="ui-sr">{word}</span>
      </span>
    </span>
  );
}
