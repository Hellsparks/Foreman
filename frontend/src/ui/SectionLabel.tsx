import type { CSSProperties, ElementType, ReactNode } from "react";

interface SectionLabelProps {
  children: ReactNode;
  /** HTML element to render as. Defaults to "span". */
  as?: ElementType;
  className?: string;
  style?: CSSProperties;
}

/** Mono 12 px uppercase label used for section headings. */
export default function SectionLabel({
  children,
  as: Tag = "span",
  className,
  style,
}: SectionLabelProps) {
  return (
    <Tag
      className={`ui-section-label${className ? ` ${className}` : ""}`}
      style={style}
    >
      {children}
    </Tag>
  );
}
