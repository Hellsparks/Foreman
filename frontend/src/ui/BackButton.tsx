interface BackButtonProps {
  label: string;
  onClick: () => void;
  /** Extra class names. */
  className?: string;
}

/** Chevron-left + label, secondary style. */
export default function BackButton({ label, onClick, className }: BackButtonProps) {
  return (
    <button
      type="button"
      className={`ui-back-btn${className ? ` ${className}` : ""}`}
      onClick={onClick}
    >
      {/* M15 5l-7 7 7 7 — square caps per design spec */}
      <svg
        width="16"
        height="16"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="square"
        aria-hidden="true"
      >
        <path d="M15 5l-7 7 7 7" />
      </svg>
      {label}
    </button>
  );
}
