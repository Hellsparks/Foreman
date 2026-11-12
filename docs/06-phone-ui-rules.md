# Phone UI rules (two items also change desktop)

The density pass shrank padding. The real problem is chrome competing with content: on 390 × 844 about 700 px is usable, and headers + tab rows + filters + button strips were eating half of it.

Rule: not "make it smaller" but "show less of it".

## 1. Chrome budget (phone)
| Element | Budget |
|---|---|
| Header | 48 px — page title + at most two icon buttons |
| Bottom bar | 56 px + safe area — five destinations |
| Everything else | **0 px by default** — filters, toolbars, sidebars, inspectors, section tabs hidden until asked for |

Content begins within 48 px of the viewport top. No repeated page heading, no description paragraph, no summary strip between header and first row of data. A screen that needs a toolbar needs a sheet or a panel instead.

## 2. Drag-out panels, not stacked sections
Secondary surfaces become edge panels with a visible grab tab:
- Left tab = navigation within the page (Library locations, Filament sections, Settings groups)
- Right tab = details for the selection (file inspector, spool inspector, printer details)
- Tab ≈ 20 × 64 px, vertically centred, chevron. Tap or drag opens; drag, backdrop tap or Escape closes.
- Panels overlay content, never push it. Max 85% viewport width. State remembered per page.
- On desktop these are the existing sidebars — same components, different presentation at the breakpoint.

## 3. Attention moves into the bell (desktop too)
- Remove the Attention strip from Fleet entirely. Fleet is printers.
- The bell + count in the top bar (desktop) or header (phone) opens a dropdown panel of open items with their actions.
- Desktop: anchored panel ≈420 px wide, max-height 70vh, scrolls. Phone: full-height sheet.
- "View all" opens the existing Attention page, which keeps history and filters.

## 4. Per page
**Library** — header: title + search + "⋯". Locations in the left panel; inspector and Send in the right. Grid starts under the header. Breadcrumbs collapse to current folder + back chevron. Upload / New folder / Paste live in "⋯". Selection mode appears only when something is selected, as a bottom action bar.

**Jobs** — cards: thumbnail, truncated name, one status chip, nothing else; the rest on open. History on phone is one row per job: outcome dot, name, relative time, grams. Stats: one number per tile, label beneath, two tiles per row.

**Filament** — section tabs move into the left panel. Header: title + search. Filters behind one "Filter" chip showing the active count. Add actions in "⋯". List edge to edge.

**Projects** — list: one card per project (thumbnail, name, progress bar, "3 of 8"), no per-plate detail. Detail page: plates as a plain list, colour slots inline as swatch + name on one line.

**Printer** — Live tab: webcam first, temperatures as one compact row, controls in a sheet behind one button.

## 5. Bottom bar
**Fleet · Jobs · Start print · Filament · More**

The middle slot was Scan; it becomes Start print (pick file → pick printer → confirm plate clear → go), the most common phone action. Scan returns under Filament when the devices exist.

## 6. Text
- Names truncate to one line with an ellipsis; two lines only for the primary item on a detail page.
- No sentence-long helper text on phone.
- Relative times in lists ("4 min ago").
- Short status chips: `▶ 63%`, `✓ Done`, `○ Offline`.

## 7. Done means
At 390 × 844 with content loaded, per page:
- content begins within 48 px of the top
- ≥ 60% of the viewport is content, not chrome, on first paint
- a list screen shows ≥ 5 rows without scrolling
- zero clipped elements, no horizontal scroll
- 44 px tap targets and focus outlines intact

Desktop stays pixel-identical except §3.

---

# Layout rules at every width (session 34e)

These apply at every width, phone and desktop.

## 8. Overlays portal to the body
- Every popover (card "⋯" menus, dropdowns, combo-box lists, tooltips, confirm popovers, the bell panel) renders through `ui/Popover` (or `ui/Menu`) into the single overlay root `#fw-overlay-root` on `<body>`. Nothing floating lives inside a page container, so no `overflow` or `transform` can clip it.
- It is `position: fixed` against its trigger, prefers below/end, flips to the other side and alignment when it would leave the viewport (8 px margin), caps its height to the room it has, and repositions on any scroll and on resize.
- Outside pointer-down or Escape closes it; Escape returns focus to the trigger. Menus move with ↑ ↓ Home End.
- Bottom sheets ("⋯" on phone, More) portal to the same root.
- No container gets `overflow: hidden/auto` just to clip rounded corners. `overflow` is only for real scrollers and for clipping images, progress fills and stripes.

## 9. Content regions fill the remaining height
- The work area is a flex column, and the page fills it. The page's content region (`.page-fill`: the list, grid, table frame or panel) takes the remaining height, so it reaches the bottom of the viewport whatever the item count: 0, 1 or 500.
- Empty space belongs inside the container, not to the page background. Empty states go inside the container.
- Pages end on the shell's bottom gutter (32 px desktop). No extra bottom padding goes under the content region.

## 10. Desktop fills the width
- No page max-widths. Only reading text keeps a measure: paragraphs `max-width: 70ch`.
- Groups of cards and sections are an auto-fit grid: `repeat(auto-fit, minmax(min(100%, <track>), 1fr))` (`.auto-cols`). They reflow to 1–4 columns, and with few items the items take the row instead of leaving empty tracks.
- Tables use the full width with proportional columns. A table gets a minimum width only where its columns would otherwise crush (640 px); below that it scrolls inside its own frame.
- Lanes (Jobs queue) share the width, each at least 272 px.

## 11. Breakpoints (from content, not round numbers)
| Value | Where | Why |
|---|---|---|
| 760 px | phone layout | Below it the rail, the two-icon header and the list rows no longer fit side by side (sessions 34b–d). |
| 1200 px | list + inspector side by side | Rail 176 + gutters 64 + the narrowest useful list (~560: a three-column table or two 280 px cards) + inspector 400. Below 1200 the inspector is the right edge panel (overlay), the same component as on phone. |
| 460 px track | form / section columns (Settings, Tools → Calibration, Service printers, Project detail, Attention cards, filament form sections) | A label + control + help column; the printer Config tab's columns. |
| 300 px track | printer cards (Fleet) | Name, chip, camera thumbnail, temperatures. |
| 280 px track | project cards | Name, due chip, progress bar. |
| 272 px | queue lane minimum | One queue card with its thumbnail, name and "⋯". |
| 160 px track | filament form field rows | A label over a numeric/text field. |
| 70ch | reading text | Comfortable line length. |

## 12. One toolbar on desktop
- A page has one toolbar. In the Library on desktop, picking files enables the toolbar's own Copy / Cut / Delete. The count plus Select all and Clear (and Copy here / Move here when the picks are in another folder) appear right-aligned in the same row. There is no second selection bar.
- On phone no toolbar is visible, so the selection actions are the bottom action bar. Never both.
