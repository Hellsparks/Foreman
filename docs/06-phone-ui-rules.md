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
