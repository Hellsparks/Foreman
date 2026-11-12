import { getCurrentWindow } from "@tauri-apps/api/window";
import { useRef, useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import type { ConnectionState } from "../hooks/useEventStream";
import { useRoute } from "../desktop/route";
import { usePhone } from "../hooks/useMediaQuery";
import AttentionBell from "../attention/AttentionBell";

/** True only when running inside the Tauri shell. */
export const IS_TAURI = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** Pages portal their title/status into this element. */
export const TOPBAR_SLOT_ID = "topbar-page-slot";
/** Phone: pages portal their own header buttons (search, "⋯") into this element. */
export const TOPBAR_ACTIONS_ID = "topbar-actions-slot";

export function WindowControls() {
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

const PAGE_TITLES: [string, string][] = [
  ["/fleet", "Fleet"], ["/printer", "Printer"], ["/jobs", "Jobs"], ["/library", "Library"],
  ["/projects", "Projects"], ["/filament", "Filament"], ["/service", "Service"],
  ["/tools", "Tools"], ["/settings", "Settings"], ["/attention", "Attention"], ["/start", "Start print"],
  ["/setup", "Setup"],
];

/** Phone header title: the section the route belongs to. */
function pageTitle(pathname: string): string {
  return PAGE_TITLES.find(([p]) => pathname === p || pathname.startsWith(p + "/"))?.[1] ?? "Foreman";
}

interface TopBarProps {
  connectionState: ConnectionState;
}

export default function TopBar({ connectionState: _connectionState }: TopBarProps) {
  const win = IS_TAURI ? getCurrentWindow() : null;
  const route = useRoute();
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const phone = usePhone();
  // Phone only: the search field is a button until tapped (see Shell.css).
  const [searchOpen, setSearchOpen] = useState(false);
  const searchRef = useRef<HTMLInputElement>(null);

  return (
    <header
      className="topbar"
      role="banner"
      data-search-open={searchOpen}
      onDragStart={(e) => e.preventDefault()}
    >
      <div id={TOPBAR_SLOT_ID} className="topbar__page" />
      {/* Phone: page title (hidden when the page puts its own header in the slot). */}
      <p className="topbar__title" aria-hidden="true">{pageTitle(pathname)}</p>

      {route?.via && (
        <span className="topbar__via" title={`${route.serverName} via ${route.address}`}>
          via {route.via}
        </span>
      )}

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
          placeholder={phone ? "Search printers, files, spools…" : "Search printers, files, spools… or type a command"}
          aria-keyshortcuts="Control+k"
          ref={searchRef}
          onBlur={() => setSearchOpen(false)}
        />
        <kbd className="topbar__kbd">Ctrl K</kbd>
      </label>

      {/* Phone: page header buttons; when present they replace the global search. */}
      <div id={TOPBAR_ACTIONS_ID} className="topbar__actions" />

      {/* Phone: search icon that expands the field to full width. */}
      <button
        type="button"
        className="topbar__search-toggle"
        aria-label="Search"
        aria-expanded={searchOpen}
        onClick={() => {
          setSearchOpen(true);
          // The field is display:none until the state applies.
          requestAnimationFrame(() => searchRef.current?.focus());
        }}
      >
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden="true">
          <circle cx="11" cy="11" r="7" />
          <path d="M20 20l-4-4" />
        </svg>
      </button>

      {/* Spacer — drag region when inside Tauri.
          data-tauri-drag-region works on Windows/X11; on Wayland we need
          startDragging() called directly from the mousedown event so the
          compositor receives the correct input serial. */}
      <div
        className="topbar__spacer"
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

      {/* Send — primary: navigate to Library */}
      <button
        type="button"
        className="topbar__btn topbar__btn--primary"
        aria-label="Send a file to a printer"
        onClick={() => navigate("/library")}
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

      {/* Attention: bell + count, opens the open items (§3, doc 06) */}
      <AttentionBell />

      {/* Window controls — Tauri only */}
      {IS_TAURI && <WindowControls />}
    </header>
  );
}
