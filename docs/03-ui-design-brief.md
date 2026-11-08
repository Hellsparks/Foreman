# Foreman — UI design brief (for Claude artifacts / Design)

Paste this whole doc into a new Design session. Attach `DESIGN.md` and `tokens.css` from https://github.com/Hellsparks/Fieldwork-designtemplate. Design **one batch per session/canvas** (§6), in order.

---

## 1. What Foreman is
A control app for a fleet of 3D printers (Klipper/Moonraker, Bambu Lab, OctoPrint, Duet) with built-in filament inventory (Spoolman-compatible). Runs as a desktop app (Tauri) and in a browser/phone against the same server. Users are makers or small print farms with 2–20 printers. Main jobs: get prints onto printers, react when prints finish, keep filament and maintenance under control, run multi-plate projects.

## 2. Visual rules (non-negotiable)
- **Fieldwork design system only.** `DESIGN.md` is the authority; use `tokens.css` semantic variables (`--fw-*`). No hardcoded hex, no other UI kit, no Tailwind look.
- Support all 4 schemes (fieldwork, vector, foundry, olive) × light/dark via `data-fw-theme` / `data-fw-mode` on root. Show at least fieldwork-dark and one light variant.
- Square panels, system sans, mono labels (12px, uppercase, tracked), sparing accent. 44px min targets. Status always text + symbol, never colour alone.
- No per-printer custom themes (old Mainsail CSS scraping is removed).

## 3. Surface rules (how interactions are built)
1. **Modal only when the task must block. Never stack dialogs.** Only two blocking dialogs exist in the whole app: *Cancel running print* and *Remove printer / delete database* (type-the-name).
2. **Long or multi-step work = a page** with URL, draft autosave and a progress checklist. No wizard modals.
3. **Selected record → Inspector**: non-modal right side sheet (~400–480px), main view stays usable. Phone: full page with back.
4. **Edit in place**: click-to-edit fields, inline "Create '…'" row inside comboboxes, explicit Save for settings groups.
5. **Undo beats confirm**: reversible actions happen immediately + toast with Undo (Fieldwork toast contract: no auto-dismiss, max 2, stable).
6. **Anything waiting on a human is an Attention item** (and a notification).
7. **Every drag has a non-drag path** (row menu / picker).
8. **Reusable panels**: *Inspector*, *Send panel*, *Load panel* are the same component wherever they appear.

## 4. Shell
**Desktop:** left rail (collapsible 176/56px, Fieldwork sidebar contract) with 8 sections: **Fleet · Jobs · Library · Projects · Filament · Service · Tools · Settings**. Top bar: **Ctrl+K search/command** field · **Scan** · **Send** (upload) · **Attention** (count) · theme. Work area: title + action row, then content. Inspector docks right.
**Phone (≤430px, Fieldwork phone reference):** bottom nav **Fleet · Jobs · Filament · More** + prominent **Scan** button. Inspector → full page with back; popovers → bottom sheets.

## 5. Shared components to define first
| Component | Contents / behaviour |
|---|---|
| **Printer card** (compact) | Name, firmware badge, state (Printing 63% · Finished – clear plate · Paused · Error · Idle · Offline), job name + ETA, nozzle/bed temps, loaded spool chips (colour + material per tool/AMS slot), small webcam thumb. ⋯ quick menu: Pause/Resume, Light, Open, Open native UI. Fixed height — no expanders. |
| **Attention item** | Icon + status word, sentence ("Benchy finished on Voron 2.4"), time, 1–3 inline actions. Variants: print finished (Good / Failed + reason chips, then **Plate cleared → start next**), error/paused, filament runout, spool low, maintenance due (Done / Snooze 10 h), printer offline, slicer upload not started (Resolve → Send panel), no compatible printer for queued job. |
| **Inspector** | Header (record identity + type label + close), sections, sticky action footer. Hosts Send panel, Load panel, Weigh, New spool. |
| **Send panel** | File summary (thumb, time, materials per tool, grams). Printer list ranked: ✔ ready · ⚠ warning with reason (wrong material, low spool, colour differs) · ✖ incompatible with reason (bed too small, nozzle, tool count, abrasive) · busy/needs plate clear. ⚠ row expands inline to fix (opens Load panel section in place). Printer with uncleared plate shows **Plate cleared** button inline; *Print now* disabled until then. Actions: **Print now** · **Queue on this printer** · **Fleet queue (any compatible)**. |
| **Load panel** | Two sides: slot (printer → tool/AMS/MMU slot) ↔ spool (search, colour swatch, material, remaining g, location). Either side can be pre-filled. Guard inline: ✖ hard block (abrasive on brass nozzle, material not allowed) needs explicit **Load anyway**; ⚠ soft (material differs from last, < 100 g) with "Weigh now". Result: toast with Undo (previous spool restored). |
| **Command palette** | Ctrl+K overlay. Mixed results: printers, files, spools (#42), filaments, projects, jobs, tasks + actions ("Send benchy…", "Add spool", "Mark belt tension done"). Recent items when empty. Keyboard-first. |
| **Toast w/ Undo**, **Badge**, **Status word**, **Spool chip**, **Grams bar** (used/reserved/left) | Fieldwork contracts. |

## 6. Screen batches (design in this order)

### Batch 1 — Shell + Fleet
- Desktop shell (rail expanded + collapsed, top bar, inspector closed/open).
- **Fleet page**: Attention strip (3–5 items, collapsible, "View all") above printer grid (8 printers, all states). Empty fleet state ("Add your first printer"). One printer offline.
- Printer card ⋯ menu (popover).
- Attention inbox full list (filter by type).
- Phone: Fleet, attention item, notification → item.

### Batch 2 — Printer page `/fleet/:id`
Tabs: **Live** (webcam large, temps with presets, progress, pause/resume/cancel, macros as favourites + all, jog rose, light, G-code console with history, "Open native UI" link) · **Queue** (this printer's lane) · **Spools** (tool/AMS slots → Load panel) · **Service** (tasks for this printer) · **History** · **Config** (sections: connection, capabilities with **Probe** results, tools/AMS, temp presets, webcams; explicit Save; Remove printer = type-name dialog).
- Cancel-print confirm dialog (the one allowed blocking dialog).
- Bambu variant (AMS 4 slots) and Klipper multi-tool variant.
- Phone Live tab.

### Batch 3 — Send flow
- Library with file selected → Inspector with file → Send panel (8 printers: 3 ✔, 2 ⚠, 2 ✖, 1 needs plate clear).
- ⚠ row expanded with inline spool swap.
- After **Fleet queue**: toast with Undo.
- Drag file onto printer card (desktop) → Send panel preselected.
- Phone: Send as full page.

### Batch 4 — Jobs
- **Queue board**: Fleet lane + one lane per printer; drag reorder/move; row ⋯ (move to…, hold, remove); held and "no compatible printer" states; job Inspector (compat per printer).
- **History**: search, filters (printer, material, outcome, project, date), count; job Inspector (file, printer, spools + grams, cost, outcome + reason, project link, **Reprint** → Send panel).
- **Finished-job outcome** inline (Good / Failed + reason chips: adhesion, spaghetti, layer shift, clog, user cancel, other).
- Stats page (utilization, failure rate per printer, grams per material).

### Batch 5 — Filament
- **Spools** list (colour, material, vendor, remaining g, location, loaded-on, reserved g); filters; spool Inspector: details, **Weigh** (live scale value → "Use 812 g"), **Load onto…**, print label, write NFC, history.
- **Load panel** from both directions (from printer slot / from spool), with hard-block and soft-warning states.
- **New spool** in Inspector: SpoolmanDB search first → prefilled; manual below; filament combobox with inline "Create 'Prusament PETG Galaxy Black'" expanding a filament section (vendor combobox same pattern); qty, weight, price, location, lot; after save: Print label · Write NFC · Load onto….
- **Filaments**, **Vendors**, **Inventory** (stock per filament, reorder threshold, reserved vs free grams), **Locations**.
- **Scan flow** (phone): camera/NFC → known spool → spool page with suggested "Load onto Bambu X1C AMS 2 (empty)"; unknown tag → New spool.

### Batch 6 — Library + Projects
- **Library**: folders + files (thumbs, slicer, time, materials), breadcrumb, "Slicer inbox" folder, upload drop zone, file Inspector.
- **Template editor page**: sections Basics · Plates (drag from side file list, reorder) · Colour slots (slot × plate matrix, preferred colour/filament) · Thumbnail; progress checklist; autosave indicator.
- **New project page**: source (template / files), name, due date, instances; per colour slot spool picker showing **grams needed vs left (minus other reservations)**; Create.
- **Project page**: click-to-edit header (name, due), progress, plates table (status, printer chip live, time, grams), plate Inspector, materials panel (slot → spool, swap inline), **Queue all remaining** (fleet queue by due date), activity log, Archive (toast Undo; Archive list has Restore).
- Projects list (active / archived filter, due soon first).

### Batch 7 — Service, Tools, Settings, Setup
- **Service**: due/overdue across fleet (task, printer, hours/grams/jobs vs interval), task Inspector (history with notes/photos, Mark done + note, Snooze 10 h, interval editor hours/grams/jobs).
- **Tools**: Calibration (pick filament + printer → print test via Send panel → enter measurements → result → Apply to filament); Swatches (2 lines of text, live 3D preview, download STL); **Import** page (upload → map fields → preview diff → apply → batch Undo); **Export** (OrcaSlicer, HueForge, CSV, JSON).
- **Settings**: single-column groups with explicit Save (General, Server & API keys, Backup, Notifications, Appearance with 4×2 theme picker, Printers list, Data).
- **Setup** pages: Mode (this machine is the server / connect to server) → Data (fresh / import Marathon / existing Spoolman DB) → Printers (discovered list + manual using Config sections) → Features & theme → Fleet with "getting started" attention items.
- **Add printer** page with discovery list.
- Command palette overlay (desktop) + Search page (phone).

## 7. Every screen must show
Default · loading (skeleton matching layout) · empty (with next action) · error (with recovery) · long names / many items (20 printers, 300 spools) · keyboard focus visible · phone layout where the screen exists on phone.

## 8. Sample data (use consistently)
Printers: **Voron 2.4 350** (Klipper, printing 63%, 1 h 12 m left, PETG) · **Bambu X1C** (AMS 4 slots, finished – needs plate clear) · **Prusa MK4** (OctoPrint, idle) · **RatRig V-Core 4** (Duet, paused – filament runout) · **Ender 3 S1** (Klipper, offline) · **Voron Trident** (Klipper, error: thermal runaway) · **Bambu P1S** (idle, AMS) · **Sovol SV08** (printing 12%).
Spools: #42 Prusament PETG Galaxy Black 812 g · #57 Polymaker PolyLite PLA Teal 120 g (low) · #61 Bambu PLA Basic White 1000 g · #63 eSun ABS+ Grey · #70 Fiberlogy PA12-CF (abrasive).
Project: "Chess set — walnut/maple", 6 plates (2 done, 1 printing, 3 pending), due Fri, slots PRIMARY (PLA Teal) + ACCENT (PLA White).
Maintenance: Belt tension (Voron, 212/200 h, overdue) · Nozzle swap (X1C, 380/400 h).

## 9. Don'ts
- No modal for editing, creating, sending or loading. No modal on top of anything.
- No browser alert/confirm.
- No expanding printer cards.
- No hover-only actions; no drag-only actions.
- Don't invent features beyond this brief; mark open questions on the canvas instead.

## 10. Hand-back
For each batch, list on the canvas: screens produced, components introduced/changed, open questions. Those notes come back to this project and feed the Claude Code frontend tasks.
