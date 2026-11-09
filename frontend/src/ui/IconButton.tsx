import type { ButtonHTMLAttributes } from "react";

interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  /** Width and height in px. Defaults to 36. */
  size?: number;
}

export default function IconButton({
  size = 36,
  style,
  children,
  type = "button",
  ...rest
}: IconButtonProps) {
  return (
    <button
      type={type}
      className="ui-icon-btn"
      style={{ width: size, height: size, ...style }}
      {...rest}
    >
      {children}
    </button>
  );
}
