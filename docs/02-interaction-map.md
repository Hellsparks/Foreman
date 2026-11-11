# Foreman rewrite — 02 Interaction map (UI flows)

Live, interactive map (current vs proposed graphs per workflow + decision points): https://claude.ai/artifact/Mb3oor9DUPN8ZSFsjLG39H

## ✅ Decisions locked (2026-11-08) — all UI decisions confirmed by owner

### Global
- **Navigation:** rail with 8 sections — Fleet · Jobs · Library · Projects · Filament · Service · Tools · Settings.
- **Inspector everywhere:** selected file/spool/job/plate/task opens in one non-modal right-side Inspector.
- **Command palette (Ctrl+K) and Scan (QR/NFC): both in v1.**
- **Destructive actions — friction ladder:** reversible → Undo toast · recoverable → light confirm · irreversible/physical → explicit confirm; printer/DB deletes = type the name.
- **Fleet page = Attention strip on top + printer grid below.**
- **Phone:** bottom nav Fleet · Jobs · Filament · More + Scan button. Inspector → full page; popovers → bottom sheets.

### Per workflow
| Workflow | Decision |
|---|---|
| Assign print | Send flow lives in the Inspector (non-modal). "Print now" blocked until the printer's last job is marked "plate cleared"; the Plate-cleared button is offered right there. Fleet queue in v1. |
| Slicer upload | Upload & print while target busy → top of that printer's queue + Attention item. |
| Queue | Foreman owns all queues (Rust scheduler) for every firmware. Fleet dispatch prefers the printer with matching filament already loaded (fallback: first compatible idle + plate cleared). |
| Print finished | "Plate cleared" via button in Attention item / notification. Outcome asked every job: one tap, skippable, defaults to "Good" after 24 h. |
| Control printer | Native UI (Mainsail/Fluidd) = link out (new window / Tauri window). No confirm on Pause. |
| Printer config | Discovered list + manual entry. |
| Load spool | Guard inline in Load panel; hard blocks need explicit "Load anyway". Drag-and-drop kept as desktop shortcut only. |
| Add stock | Inline create inside comboboxes. SpoolmanDB search first, manual below. |
| Weigh | No auto-accept; user taps "Use 812 g". |
| Template | Single editor page with sections + progress checklist (autosave). |
| New project | Reserve filament: reserved grams shown in inventory, released as plates finish. |
| Edit project | Archive = Undo toast + Restore from archive. |
| Print plates | "All remaining" → fleet queue ordered by due date. |
| Service | Mark done straight from notification, with Undo. |
| Import/export | Whole import can be undone (batch). |
| Bambu accounts (D11, 2026-11-10) | Settings → Bambu accounts: list of signed-in accounts (label, region, state, printer count), Add account → Bambu sign-in, Import printers (discovered list, tick to add), Sign out with light confirm. On ARM servers the section shows one warning ("Bambu Cloud needs an x86_64 server") and no Add button. Expired sign-in → Attention item with a Sign in action. |

## Surface rules (apply to every screen)
1. Modal only when the task must block. Never stack dialogs.
2. Long or multi-step work = a page with a URL and a draft (no wizard modals).
3. Selected record → Inspector (non-modal right side sheet). Phone: full page with back.
4. Edit where it is shown: click-to-edit, inline create inside comboboxes, explicit Save for settings groups.
5. Undo beats confirm. Reversible → act + Undo toast. Irreversible/physical (cancel running print, delete forever) → specific confirm; printer/DB removal = type name.
6. Anything waiting on a human → Attention item + notification.
7. Every drag has a non-drag path (menus/pickers for touch + keyboard).
8. One component per job: **Send panel**, **Load panel**, **Inspector** reused across all entry points.

## Backend implications of the UI decisions (for Claude Code)
- Soft delete + undo endpoints (files, queue items, spool assignment, plates, maintenance done, archive) and batch-id on imports.
- Job outcome field + failure reasons; auto-"Good" after 24 h job.
- Printer state `needs_plate_clear` gates start + scheduler.
- Filament reservations table (project ↔ spool ↔ grams), released per finished plate.
- Scheduler dispatch scoring: compatibility → loaded-filament match → idle + plate cleared → due date.
- Attention items as a first-class entity (type, target, actions, created/resolved) + push to WS + notifications.
- Command palette search endpoint across printers, files, spools, filaments, projects, jobs, tasks + action registry.

## Workflows mapped (blocking dialogs today → proposed)
Assign print 2→0 · Slicer upload 0→0 · Queue 0→0 · Finish/clear plate 0→0 · Control printer 1→1 · Printer config 3→1 · Load spool 2→0 · Add stock 3→0 · Weigh 0→0 · Create template 5→0 · Create project 3→0 · Edit project 4→0 · Print plates 0→0 · Service 0→0 · History 0→0 · Import/export 4→0 · Calibration 0→0 · Setup 1→0. Total 28 → 2.

## Bugs found in current code while tracing
- Project plate print sets only the first colour slot's spool (`ProjectDetailView.handlePrint` uses `color_assignments[0]`), no compat check, no guard, uses `alert()`.
- Slicer upload-and-print auto-starts with no compat/plate-clear check; if printer busy it is silently not started (console log only).
- Project archive uses `window.confirm()`.
- Spool drag-assign is disabled on mobile with no alternative path.
