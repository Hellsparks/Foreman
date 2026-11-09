import type { ButtonHTMLAttributes } from "react";

export type ButtonVariant = "primary" | "secondary" | "small" | "danger";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  /** Pin the button to an exact pixel width (e.g. for aligned action columns). */
  fixedWidth?: number;
}

export default function Button({
  variant = "secondary",
  fixedWidth,
  style,
  children,
  type = "button",
  ...rest
}: ButtonProps) {
  return (
    <button
      type={type}
      className={`ui-btn ui-btn--${variant}`}
      style={fixedWidth != null ? { width: fixedWidth, minWidth: fixedWidth, ...style } : style}
      {...rest}
    >
      {children}
    </button>
  );
}
