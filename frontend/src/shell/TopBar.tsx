import { getCurrentWindow } from "@tauri-apps/api/window";
import type { ConnectionState } from "../hooks/useEventStream";

/** True only when running inside the Tauri shell. */
const IS_TAURI = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** Pages portal their title/status into this element. */
export const TOPBAR_SLOT_ID = "topbar-page-slot";

function WindowControls() {
  const win = getCurrentWindow();

  return (
    <div className="topbar__wc" role="group" aria-label="Window controls">
      {/* Minimize */}
      <button
        type="button"
        className="topbar__wc-btn"
        aria-label="Minimize"
        onClick={() => win.minimize()}
      >
        <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
          <line x1="1" y1="5" x2="9" y2="5" />
        </svg>
      </button>

      {/* Maximize / restore */}
      <button
        type="button"
        className="topbar__wc-btn"
        aria-label="Maximize or restore"
        onClick={() => win.toggleMaximize()}
      >
        <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
          <rect x="1" y="1" width="8" height="8" />
        </svg>
      </button>

      {/* Close */}
      <button
        type="button"
        className="topbar__wc-btn topbar__wc-btn--close"
        aria-label="Close"
        onClick={() => win.close()}
      >
        <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
          <line x1="1" y1="1" x2="9" y2="9" />
          <line x1="9" y1="1" x2="1" y2="9" />
        </svg>
      </button>
    </div>
  );
}

interface TopBarProps {
  connectionState: ConnectionState;
  attentionCount: number;
}

export default function TopBar({ connectionState: _connectionState, attentionCount }: TopBarProps) {
  const win = IS_TAURI ? getCurrentWindow() : null;

  return (
    <header className="topbar" role="banner" onDragStart={(e) => e.preventDefault()}>
      <div id={TOPBAR_SLOT_ID} className="topbar__page" />

      {/* Search field */}
      <label className="topbar__search" aria-label="Search or run a command">
        <svg
          width="18"
          height="18"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
          aria-hidden="true"
          style={{ flexShrink: 0 }}
        >
          <circle cx="11" cy="11" r="7" />
          <path d="M20 20l-4-4" />
        </svg>
        <input
          type="search"
          className="topbar__search-input"
          placeholder="Search printers, files, spools… or type a command"
          aria-keyshortcuts="Control+k"
        />
        <kbd className="topbar__kbd">Ctrl K</kbd>
      </label>

      {/* Spacer — drag region when inside Tauri.
          data-tauri-drag-region works on Windows/X11; on Wayland we need
          startDragging() called directly from the mousedown event so the
          compositor receives the correct input serial. */}
      <div
        data-tauri-drag-region
        style={{ flex: 1, cursor: IS_TAURI ? "grab" : undefined }}
        onMouseDown={
          win
            ? (e) => {
                if (e.button !== 0 || e.detail === 2) return;
                e.preventDefault();
                win.startDragging();
              }
            : undefined
        }
        onDoubleClick={win ? () => win.toggleMaximize() : undefined}
      />

      {/* Send — primary */}
      <button
        type="button"
        className="topbar__btn topbar__btn--primary"
        aria-label="Upload a file to send to a printer"
      >
        <svg
          width="18"
          height="18"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          aria-hidden="true"
        >
          <path d="M12 16V4M7 9l5-5 5 5M4 20h16" />
        </svg>
        Send
      </button>

      {/* Attention — accent border with count */}
      <button
        type="button"
        className="topbar__btn topbar__btn--attention"
        aria-label={`Attention — ${attentionCount} item${attentionCount === 1 ? "" : "s"} need action`}
      >
        <svg
          width="18"
          height="18"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
          aria-hidden="true"
        >
          <path d="M6 16V11a6 6 0 0112 0v5l2 2H4l2-2zM10 21h4" />
        </svg>
        <span className="topbar__attention-count">{attentionCount}</span>
      </button>

      {/* Window controls — Tauri only */}
      {IS_TAURI && <WindowControls />}
    </header>
  );
}
