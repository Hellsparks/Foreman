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

## Themeable tokens (Phase 6.1) — use these, never the literal
A user theme (Settings → Appearance → Import) can change every token below, so a literal in CSS or an inline style is a bug: it will not follow the theme. Defaults equal the pre-6.1 literals. Defined in `frontend/src/theme/theme.css` (spacing 1–4 and 6 come from `design/tokens.css`, which stays unmodified).

| Token | Default | Use for |
|---|---|---|
| `--fw-radius` | 2px | every `border-radius` that was 2 px (buttons, fields, chips with corners). `--fw-radius-control` follows it. |
| `--fw-border-width` | 1px | every 1 px border: `border: var(--fw-border-width) solid var(--fw-border)`. Focus outlines stay 2 px. |
| `--fw-control-h` | 44px | standard button / field / touch target height; `min-width: 44px` touch targets. `--fw-control-height` follows it. |
| `--fw-control-h-md` | 40px | buttons in lists and rows |
| `--fw-control-h-sm` | 36px | small buttons, inline controls |
| `--fw-space-1` … `--fw-space-6` | 4, 8, 12, 16, 20, 24 px | gap, padding, margin. Odd values (6, 10, 14 px) are still literals; use the nearest token in new code. |
| `--fw-rail-w` | 176px | sidebar rail width (expanded) |
| `--fw-font-body` | Arial, Helvetica, sans-serif | body text; `--fw-sans` follows it, and so do `<button>`, `<input>`, `<select>` |
| `--fw-font-mono` | ui-monospace, SFMono-Regular, Consolas, monospace | labels, chips, numbers; `--fw-mono` follows it. Must have tabular figures. |
| `--fw-font-size` | 14px | UI text size (anything that was 14 px) |
| `--fw-line-height` | 1.5 | body line height where it was 1.5 |
| colours | per scheme | the semantic `--fw-*` colours in `tokens.css` (background, surface, raised, text, muted, border, accent, accent-hover, on-accent, button-hover, focus, success/warning/danger/info and their `-bg`) |

Inline styles: `style={{ gap: "var(--fw-space-2)", borderRadius: "var(--fw-radius)" }}`, not `gap: 8`.

Theme file format (validated on import; unknown keys rejected; contrast below 4.5:1 text / 3:1 UI and a mono font without tabular figures are warnings, not blocks):
```json
{
  "name": "Compact rounded (test)",
  "tokens": { "--fw-radius": "8px", "--fw-font-body": "\"My Sans\", Arial, sans-serif" },
  "fonts": [{ "family": "My Sans", "file": "my-sans.woff2", "weight": "400" }]
}
```
Font files go in `<data_dir>/fonts/` and are served at `/theme/fonts/<file>` (`.woff2` only). `<data_dir>/custom.css` is served at `/theme/custom.css` and loaded after the theme when the Settings toggle is on — **unsupported**: class names may change in any update. Sample theme: `frontend/public/themes/compact-rounded.json` (also served at `/themes/compact-rounded.json`).

**Longest labels** (Settings → Appearance) renders worst-case strings in the real components and flags anything that clips; "Scan this page too" checks the rest of the open page. Run it after any font, spacing or layout change.

## Design rules (from the UI workshop — apply everywhere)
1. **Fieldwork tokens only.** No new colours. Status colours: `--fw-success/-warning/-danger/-info` + `-bg`.
2. **Buttons**: min-height 44 px (36 px small, 40 px in lists). Standard min-width 120 px; small 96 px. Buttons in a row of actions share one fixed width (Attention actions: 148 px, grid column layout) so they line up across rows. Primary = `--fw-accent-hover` bg + `--fw-on-accent` text + monospace bold. Secondary = 1 px `--fw-muted` border, transparent. Radius 2 px.
3. **Actions are buttons, not blue links.** Links only for real navigation in text.
4. **Back/return buttons**: chevron SVG (`M15 5l-7 7 7 7`, square caps) + label, secondary style. No "←" characters.
5. **External/open-native**: arrow-out SVG (`M14 4h6v6M20 4l-9 9M18 14v6H4V6h6`), not "↗".
6. **Labels/section heads**: monospace 12 px, uppercase, letter-spacing .06em, `--fw-muted`.
7. **Status chips**: monospace 12 px bold, symbol + word (▶ Printing, ✓ Finished, ‖ Paused, ✖ Error, ○ Offline, ● Idle), coloured fg on its `-bg`.
8. **Filament swatches**: single colour = square/circle fill; coextruded (multi_color_direction=coaxial) = split circle; gradient (longitudinal) = gradient bar; empty slot = dashed border.
9. Font: body 14 px / 1.5 (`--fw-font-body`, `--fw-font-size`, `--fw-line-height`; Arial/Helvetica by default); headings tight letter-spacing (-0.025em); h1 32 px.
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
# docs/ui-ref — Phase 4 additions (append to README.md)

## Filament — `Filament.dc.html` (899 lines, big: read only the part you need)
Tabs: Spools · Filaments · Inventory · Vendors · Other (locations etc).

| Lines | What |
|---|---|
| 1–56 | styles + shell |
| 57–64 | tab bar |
| 65–121 | **Spools** list: filters, search, colour swatches, remaining bar, rows |
| 122–147 | **Filaments** list |
| 148–195 | **Inventory**: tracked filaments, target/buy-below, OK/BUY/OUT, summary tiles, shopping list |
| 196–208 | **Vendors** |
| 209–211 | Other |
| 222–260 | right panel: spool inspector (`mSpool`) — weigh, load, measure, edit |
| 261–398 | **filament form** (`mFil`) — vendor, material + modifier, abrasive, colour type (single / coextruded / gradient), TD, collapsing sections |
| 399–420 | vendor form (`mVend`) |
| 421–442 | add stock (`mStock`) |
| 443–554 | **new spool flow** (`mNew`): search existing → form → create |
| 555–899 | demo logic; `renderVals()` from 578 = behaviour spec |

Rules that came out of the UI workshop and must hold:
- Dropdowns are inline, not modal.
- Sections in the right panel collapse as fields are filled.
- No weight field when adding a *filament*; no TD1 box when adding a *spool* — those live inside the boxes that need them.
- Abrasive is an explicit flag, auto-ticked for CF/GF/AF/Metal/Glow modifiers, user-overridable.
- Swatches: single = filled circle; coextruded = split circle; gradient = bar; empty = dashed.
# docs/ui-ref — Phase 5 additions (append to README.md)

Five boards. Read only the lines you need; `renderVals()` in each is the behaviour spec.

| File | Lines | What |
|---|---|---|
| `Projects.dc.html` (313) | 47–71 | project list |
| | 72–244 | project detail: plates, colour slots, per-file copies, progress, live plate status, archive |
| | 245+ | logic |
| `ProjectNew.dc.html` (261) | 1–178 | new project: from template or loose files, auto-grouped colour slots, spool per slot, due date, instances |
| | 179+ | logic |
| `TemplateEditor.dc.html` (246) | 1–207 | template editor: plates, colour slots, plate↔slot map, copies per set, categories, thumbnail, swap options |
| | 208+ | logic |
| `Service.dc.html` (158) | 1–113 | maintenance: due work, intervals, mark done, history |
| | 114+ | logic |
| `Tools.dc.html` (327) | 48–55 | tab bar |
| | 56–141 | **Calibration**: skew (printer) and shrinkage (filament) as separate flows |
| | 142–162 | swatch generator |
| | 163–187 | import |
| | 188–264 | export: OrcaSlicer profiles, HueForge catalogue |
| | 265+ | logic |

Phase 5 rules from the workshop:
- Copies are per file/plate, editable on a running project, never below what's already printed or printing. Grams, reservations, progress and the fleet queue count prints, not plates.
- Skew is a printer property (Klipper `[skew_correction]`); shrinkage is a filament property stored in `orcaslicer_config` as `shrinkage_xy` / `shrinkage_z`. Two separate flows, never mixed.
- A project built from loose files groups tool+colour across files into shared colour slots, which the user can split or merge.
