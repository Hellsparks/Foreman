# docs/ui-ref — UI reference from the Foreman UI demo

These files are exported from the Foreman UI demo (claude.ai design artifact). They are the visual spec. Match them; don't copy the markup 1:1.

## How to read a `.dc.html` file
- The `<style>` block in `<helmet>` holds the Fieldwork tokens (`--fw-*`) for all 4 schemes × light/dark. The repo already has these in `frontend/src/design/tokens.css`. Use those, not new values.
- Markup between `<x-dc>` tags is the screen. `{{name}}` is a value from `renderVals()` in the `<script type="text/x-dc">` block at the bottom.
- `<sc-for list="{{x}}" as="i">` = map over a list. `<sc-if value="{{x}}">` = conditional.
- The script's `renderVals()` shows the demo data and the interactions (what each button does). Treat that as behaviour spec, not code to port.
- Inline styles are deliberate: they show exact spacing, sizes and borders. In React, turn them into CSS classes or CSS modules.

## Files and what they define
| File | Use it for |
|---|---|
| `Main.dc.html` | **The shell**: sidebar rail (176 px, collapses to 56 px, icons + labels, active marker = 3 px accent bar + raised bg), top bar (search 44 px with Ctrl K chip, Scan, Send = primary, Attention count). **Fleet**: header line, Attention strip, printer card grid (300 px min, 252 px tall), card menu, right-hand inspector, toasts with Undo. |
| `Attention.dc.html` | Full Attention page: filter buttons (joined), item rows, fixed-width action buttons, resolved list. |
| `FleetEmpty.dc.html` | Empty Fleet: getting-started checklist, printers found on the network. |
| `PhoneFleet.dc.html` | Phone layout (390 px): bottom tabs, compact cards. |
| `Connect.dc.html` | Connect / sign in / backup routes / lost connection (Phase 2 end, not UI 1). Shows the custom window frame. |

## Design rules (from the UI workshop — apply everywhere)
1. **Fieldwork tokens only.** No new colours. Status colours: `--fw-success/-warning/-danger/-info` + `-bg`.
2. **Buttons**: min-height 44 px (36 px small, 40 px in lists). Standard min-width 120 px; small 96 px. Buttons in a row of actions share one fixed width (Attention actions: 148 px, grid column layout) so they line up across rows. Primary = `--fw-accent-hover` bg + `--fw-on-accent` text + monospace bold. Secondary = 1 px `--fw-muted` border, transparent. Radius 2 px.
3. **Actions are buttons, not blue links.** Links only for real navigation in text.
4. **Back/return buttons**: chevron SVG (`M15 5l-7 7 7 7`, square caps) + label, secondary style. No "←" characters.
5. **External/open-native**: arrow-out SVG (`M14 4h6v6M20 4l-9 9M18 14v6H4V6h6`), not "↗".
6. **Labels/section heads**: monospace 12 px, uppercase, letter-spacing .06em, `--fw-muted`.
7. **Status chips**: monospace 12 px bold, symbol + word (▶ Printing, ✓ Finished, ‖ Paused, ✖ Error, ○ Offline, ● Idle), coloured fg on its `-bg`.
8. **Filament swatches**: single colour = square/circle fill; coextruded (multi_color_direction=coaxial) = split circle; gradient (longitudinal) = gradient bar; empty slot = dashed border.
9. Font: Arial/Helvetica body 14 px / 1.5; headings tight letter-spacing (-0.025em); h1 32 px.
10. Every interactive element has a visible `:focus-visible` outline (`--fw-focus`).

## Printer page (UI 3)
`Printer.dc.html` — big file (≈115 KB, 917 lines). Read only the part you need:
- lines 1–68: header (back button, name, state, connection path) + tab bar
- lines 69–217: **Live tab** (webcams, temps with editable targets + presets, movement, console, macros)
- lines 218–291: Queue / Spools / Service / History tabs
- lines 292–381: Files · SSH tab (SFTP browser + terminal + shell macros)
- lines 382–513: Config tab (connection paths, SSH, settings)
- lines 514–917: demo logic (`renderVals()` from line 688 has the behaviour)
# docs/ui-ref — Phase 3 additions (append to README.md)

## Library (file browser + Send) — `Library.dc.html` (465 lines)
- lines 1–110: styles incl. the Fieldwork `.fw-browser` CSS (flat view only), shell
- lines 111–142: browser body — pick/carry banner ("Copy here / Move here"), conflict dialog (Keep both / Replace / Skip), new-folder row, grid cards with previews, empty state
- lines 143–190: right panel — file inspector (preview, filament, time, grams, fits N printers, actions) and the **Send panel** (per-printer compatibility rows + Print now / Queue / Fleet queue)
- lines 191–199: Transfers list + footer
- lines 200–465: demo logic (`renderVals()` from line 251 = behaviour)

## Jobs — `Jobs.dc.html` (385 lines)
- lines 61–104: Queue board (per-printer columns + fleet queue)
- lines 105–141: History (filters, outcome rating)
- lines 142–209: Stats
- lines 210–385: demo logic (`renderVals()` from line 274)
