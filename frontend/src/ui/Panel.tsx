import type { ReactNode } from "react";

interface PanelProps {
  /** Mono uppercase header text. Omit to render no header bar. */
  header?: string;
  /** Elements rendered on the right of the header (e.g. action buttons). */
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}

export default function Panel({ header, actions, children, className }: PanelProps) {
  const showHeader = header !== undefined || actions !== undefined;
  return (
    <div className={`ui-panel${className ? ` ${className}` : ""}`}>
      {showHeader && (
        <div className="ui-panel__header">
          {header !== undefined && (
            <span className="ui-panel__title">{header}</span>
          )}
          {actions !== undefined && (
            <div className="ui-panel__actions">{actions}</div>
          )}
        </div>
      )}
      <div className="ui-panel__body">{children}</div>
    </div>
  );
}
