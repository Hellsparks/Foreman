# Foreman rewrite — 01 Workflows & backend map

Source audited: `Hellsparks/Foreman-overview` (README still says "Marathon"), `Hellsparks/Fieldwork-designtemplate`, `Donkie/Spoolman` master (pyproject 0.27.1, 2026-11-02).
Rule: **if it isn't frontend, it's Rust.** (One exception: the C++ glue in `foreman-bambu-helper`, D11.) Frontend = Vite + React + TS, styled only from Fieldwork `tokens.css` / `DESIGN.md`.

---

## 0. Architecture decisions

**Status 2026-11-08:** D1, D3, D4, D6, D8, D9 confirmed by owner. v1 firmware: Klipper/Moonraker, OctoPrint, Duet, Bambu (built last). Bambu driver: two auth modes — Developer Mode (unsigned MQTT/FTPS) and signed mode for normal-firmware printers (RSA-signed MQTT using a user-supplied cert/key, loaded from config, not committed to the repo). Auth default: single admin + API keys (not yet explicitly confirmed).

**Status 2026-11-10:** D11 added by owner — Bambu Cloud the Orca way, several Bambu accounts at once. Printers stay cloud-bound (Bambu Handy and remote access keep working); Bambu's own plugin signs commands. Bambu Cloud needs an x86_64 server; ARM servers show a warning and keep LAN/Developer Mode only. The user-supplied cert/key signed mode above stays a later option, not the default.

| # | Decision | Recommendation | Why |
|---|---|---|---|
| D1 | Where the backend runs | **Rust workspace, one core, three shells**: `foreman-server` (headless axum, systemd/Docker on Pi/NAS), `foreman-desktop` (Tauri 2, embeds the same server *or* connects to a remote one), Tauri mobile later | A fleet manager must be always-on: queue auto-start, job logging, Moonraker→Spoolman websocket, slicer uploads. A desktop app that closes kills all of that. Tauri 2 targets Android/iOS too. |
| D2 | Frontend ↔ backend transport | **HTTP + one WebSocket event stream for all data**, Tauri `invoke` only for native things (file dialogs, tray, notifications, server discovery) | Same Vite build works in browser, desktop, phone. No duplicated API. |
| D3 | Spoolman | **`spoolman-rs` crate**, drop-in for `/api/v1` incl. websockets, runs on an *existing* Spoolman DB unmodified (reads `alembic_version`, refuses newer unknown revisions) | Moonraker/Mainsail/Fluidd/KlipperScreen/OctoPrint-Spoolman keep working pointed at it. |
| D4 | Spoolman DB engines | SQLite + PostgreSQL + MySQL/MariaDB via `sqlx`; CockroachDB = "best effort via Postgres driver" | Upstream supports all five; sqlx covers PG/MySQL/MariaDB/SQLite. |
| D5 | Spoolman compatibility gate | Run **upstream `tests_integration/` suite** against spoolman-rs (it's black-box HTTP via `SPOOLMAN_HOST`/`SPOOLMAN_PORT`) | Free, authoritative test oracle. Pin to one upstream tag. |
| D6 | Theming | ✅ **DECIDED:** drop per-printer Mainsail CSS scraping + community Mainsail themes. Only Fieldwork's schemes (fieldwork / vector / foundry / olive) × light/dark, via `data-fw-theme` / `data-fw-mode` on root. Delete `scopeCSS`, `themeScraper`, `scrapedCssCache`, `scrape_css_path` column, `/printers/scrape-theme`, themes feature. | Decided 2026-11-08. |
| D7 | Status updates | Moonraker: **websocket subscriptions**, not 3 s polling. Bambu: persistent MQTT. OctoPrint/Duet: polling stays (per-driver interval) | Lower latency, less load, accurate job end detection. |
| D8 | Swatch STL generator | **Port to Rust** (`foreman-swatch` crate), see §5 | Job is tiny: fixed plate + 2 lines of debossed text. Python/CadQuery (~500 MB) is overkill. |
| D9 | Docker self-management (install Spoolman, GHCR self-update, socket mount) | **Remove.** Spoolman is built in; updates handled by desktop updater / server package. Research update mechanism at that phase | Docker-socket-in-container is a big attack surface and no longer needed. |
| D10 | Marathon data | ❌ **DROPPED (owner, 2026-11-12):** no Marathon importer. The owner was the only Marathon user and the old data isn't needed. Foreman starts clean; the live Spoolman DB is used as is (W1 "use existing Spoolman DB"). | ~~One-shot importer for `marathon.db` + uploads dir.~~ |
| D11 | Bambu Cloud | ✅ **DECIDED 2026-11-10:** do what OrcaSlicer does — load Bambu's prebuilt `bambu_networking` plugin (downloaded from Bambu, pinned ABI series; a custom path such as Open Bamboo Networking is allowed). One `foreman-bambu-helper` process per Bambu account (the plugin keeps one login per process), talking JSON lines over stdio to the server. Only the helper has C++ (one glue file). **x86_64 only** (Bambu publishes the plugin for x86_64): on ARM the feature is off with a warning. | Keeps printers in cloud mode with Handy working, gets signing from Bambu's own plugin, and lets several people's Bambu accounts sit next to the Klipper printers in one Foreman. Orca itself can only hold one Bambu account. |

---

## 1. Workflows (user jobs, current → rewritten)

Each workflow lists: trigger · steps · backend pieces · what changes vs today.

### W1 — First run / setup
- **Trigger:** no config.
- **Steps:** choose mode (this machine is the server / connect to a server) → data source (fresh · use existing Spoolman DB · point at external Spoolman; no Marathon import, D10 dropped) → add printers (auto-discover + manual) → optional features → done.
- **Backend:** `setup` state machine, Spoolman DB detector (engine + alembic revision), printer discovery.
- **Changes:** Spoolman install step gone (built-in). Add discovery (mDNS for Moonraker, SSDP/LAN for Bambu — verify at build). Wizard resumable.

### W2 — Add / configure a printer
- **Steps:** pick firmware (Klipper/Moonraker, Bambu LAN, Bambu Cloud account, OctoPrint, Duet) → connection test → capabilities auto-read where possible (bed size, nozzle, extruder count, MMU/AMS, webcams, macros) → manual: abrasive-capable, tool slots, temp presets, sort order, group/tags.
- **Backend:** `PrinterDriver` trait, capability probe per driver, presets, webcam registry.
- **Changes:** capability *probe* instead of hand-entry; printer **groups/tags** (needed for fleet queue W5).
- **Bambu Cloud (D11):** Settings → Bambu accounts → sign in (one or more accounts) → import that account's printers. Each imported printer belongs to one account. On an ARM server this section shows the x86_64 warning instead.

### W3 — Monitor the fleet
- **Steps:** dashboard grid/list → per-printer state, temps, progress, ETA, active spool(s), webcam → drill into printer.
- **Backend:** event bus (`tokio::broadcast`) → `/ws/events` stream; status cache; webcam proxy (MJPEG/WebRTC passthrough; Bambu camera stream).
- **New:** **Attention inbox** — one list of everything that needs a human: print finished (clear plate), error/paused, filament runout, spool low, maintenance due, offline printer, failed upload. This becomes the home screen's main block and the notification source.

### W4 — Get a file onto a printer
- **Entry points:** drag-drop upload · slicer "upload / upload & print" (OctoPrint-compatible endpoint + API key) · file library · project plate.
- **Steps:** upload → parse G-code (slicer, printer model, nozzle, materials per tool, filament g/mm per tool, time, thumbnails) → **compatibility check** against every printer (bed, nozzle, tool count, material, abrasive, loaded filament colour/material) → choose: Print now / Add to printer queue / Add to fleet queue.
- **Backend:** file store + folders, streaming G-code parser, thumbnail extractor, compatibility engine (one Rust function used by UI, MCP, queue scheduler).
- **Changes:** one unified **"Send" flow** everywhere instead of separate SendToPrinterModal / queue / project paths. Compatibility shown as ✔ / ⚠ (reason) / ✖ per printer.

### W5 — Queue
- **Today:** per-printer queue with auto-start.
- **Rewrite:** per-printer queues **plus a fleet queue** ("next compatible free printer"). Scheduler picks printer by compatibility + loaded filament + idle.
- **Safety gate:** auto-start only after **"plate cleared" confirmation** (button in Attention inbox / phone notification / optional macro).
- **Backend:** `scheduler` task reacting to events (printer idle, plate cleared, job added).

### W6 — Control a printer
- Start/pause/resume/cancel, set temps + presets, macros, movement rose, light, G-code console, Mainsail/Fluidd "open native UI" (embedded webview as escape hatch).
- **Backend:** driver commands, command audit log.
- **Changes:** cancel/stop always confirm; console history; macro favourites per printer.

### W7 — Job lifecycle (automatic)
- **Detect** start/end/cancel/error per driver → write job record → **filament deduction** → mark project plate → accrue maintenance hours → raise Attention item.
- **Critical rule:** avoid double deduction. Moonraker with `[spoolman]` configured reports usage itself; for Bambu/OctoPrint/Duet (or Moonraker without it) Foreman deducts from parsed G-code per tool/AMS slot. Setting per printer: `usage_reporter = moonraker | foreman`.
- **Backend:** `job_engine` (port of `jobLogger.js` + `poller.js` terminal-state logic), uses `test-gcodes/` scenarios as fixtures.

### W8 — Filament & spools (Spoolman-backed)
- **Load spool:** pick printer → tool / AMS / MMU slot → **filament guard** (material vs printer, abrasive vs nozzle, Bambu tray warnings, low remaining) → assign (sets Moonraker active spool / Bambu tray mapping).
- **Identify spool:** QR label scan, **NFC tag** (Spoolman now has tags + scan relay — expose it), search by colour (hex distance), location.
- **Weigh / measure:** Teamster load-cell scale (live weight, tare, calibrate) → `PUT /spool/{id}/measure`.
- **Inventory:** stock per filament, reorder thresholds, storage locations, archive empty.
- **Add stock:** vendor / filament / spool, SpoolmanDB external import, clone, multi-colour, price + currency convert, CSV/JSON import with field mapping, colorimeter panel.
- **Backend:** spoolman-rs (data) + `filament` module (guard, inventory rules, scale proxy, Bambu tray sync).
- **Changes:** "Load spool" as one flow reachable from printer, spool, and scan.

### W9 — Projects (templates → runs → archive)
- **Template:** name, plates (copied G-code), colour slots (PRIMARY/ACCENT/…), plate↔slot map, categories, thumbnail, swap options.
- **Start project:** pick template → assign spool per slot (search / hex match) → due date → instances count.
- **Run:** each plate goes through W4/W5 (can fan out across printers) → auto-marked by W7 → progress, time, grams, cost.
- **Archive:** read-only record.
- **Changes:** "Print all remaining plates" → fleet queue. Project shows which plate is on which printer live.

### W10 — Calibration
- Shrinkage (Calistar / fleur_de_cali port) → per-axis correction → saved to the filament's `orcaslicer_config` extra field as `shrinkage_xy` / `shrinkage_z` (percent, stored as strings) → feeds Orca export (`filament_shrink` / `filament_shrinkage_compensation_z`). `material_modifier` holds modifiers (CF, Silk), not shrinkage. *(corrected session 33)*
- Pure math → Rust module with unit tests ported from current page.

### W11 — Exports / generators
- OrcaSlicer filament profiles (incl. Bambu filament IDs), HueForge catalogue (TD values via `hue_td` field), swatch STL (D8).
- Marathon's Spoolman extra fields stay as Spoolman extra fields: `url`, `hue_td`, `swatch`, `orcaslicer_config`, `material_modifier`.

### W12 — Maintenance
- Tasks with interval (print hours) per printer → hours accrue from W7 → due/overdue → Attention item → mark done (logs date, resets counter).
- **Add:** interval by filament grams or job count, notes/photos per service.

### W13 — History & statistics
- Job history (filters: printer, material, spool, project, outcome), utilization, fleet insights, cost per job/project, failure rate per printer.
- Backend: SQL aggregates, CSV export.

### W14 — System
- Settings, API keys (slicer + MCP + remote clients), **auth** (needed once server is reachable from phone), backup (scheduled, local + SMB, includes Spoolman DB), restore, DB export/import, logs, bug report, updates.
- **MCP server:** move into Rust server as an endpoint; keep all current tools (`list_printers` … `create_vendor`, ~30).

### W15 — Remote / phone
- Same frontend, responsive (Fieldwork phone reference). Priority on phone: Attention inbox, clear plate, pause/cancel, webcam, scan & load spool, weigh spool.

---

## 2. Backend module map (Rust workspace)

```
crates/
  foreman-core/      domain types, sqlx SQLite DB + migrations, event bus, settings, auth
    drivers/         PrinterDriver trait + moonraker (WS JSON-RPC), bambu (MQTT/TLS), octoprint, duet
    files/           store, folders, gcode parser, thumbnails
    compat/          compatibility engine (W4/W5/W8 guard)
    jobs/            job engine, usage deduction (W7)
    scheduler/       per-printer + fleet queue (W5)
    projects/        templates, projects, plates (W9)
    filament/        guard, inventory, scale proxy, Bambu tray sync (W8)
    maintenance/ stats/ calibration/ exports/ backup/
  spoolman-rs/       Spoolman-compatible API + WS, multi-DB, alembic-aware (D3–D5)
  foreman-server/    axum: REST, /ws/events, static frontend, OctoPrint-compat, MCP, Spoolman mount
  foreman-desktop/   Tauri 2 shell: embedded server or remote, tray, notifications, dialogs
  foreman-bambu-helper/  one process per Bambu account: loads Bambu's network plugin via one C++ glue file (D11)
frontend/            Vite + React + TS, Fieldwork tokens, API types generated from Rust
```

Cross-cutting: typed API (Rust → TS type generation, tool picked at scaffold step), `tracing` logs, integration tests with mock Moonraker/Bambu/OctoPrint/Duet servers replaying `test-gcodes/` scenarios.

---

## 3. Build order for Claude Code

1. **Scaffold** workspace, CI, lint, type-gen pipeline, fish-safe dev commands.
2. **spoolman-rs** — isolated, has the upstream test suite as oracle. Done = suite green on SQLite + Postgres + MariaDB, Moonraker connects and reports usage.
3. **Core + drivers + event stream** (W2, W3, W6) with mock printers.
4. **Files, compat, queue, job engine** (W4, W5, W7).
5. **Filament flows** (W8) on top of spoolman-rs.
6. **Projects, maintenance, stats, calibration, exports** (W9–W13).
7. **System**: setup, backup, auth, MCP (W1, W14). (Marathon importer dropped, D10.)
8. **Tauri desktop shell**, then mobile (W15).
9. **Frontend per workflow** — UI specs come from the Claude artifacts design track; this doc defines the flows each screen must serve.

---

## 4. Open questions
- Auth model: defaulting to single admin + API keys (slicer, MCP, remote clients). Confirm.
- Fleet queue + Attention inbox: confirmed in v1 (see doc 02).

---

## 5. Swatch generator port (D8)

**What it does today** (`backend/src/services/swatch_generator.py`): load `swatch.step` (75 × 40 × 3 mm plate), deboss two text lines 0.6 mm deep (upper band Y 30–38, lower band Y 21–29, left inset X 8.5, max width 62 mm, font 6.4 mm, Arial), export STL. Frontend previews with three.js.

**Rust plan (`crates/foreman-swatch`):**
1. One-time: convert `swatch.step` → base mesh, commit it as an asset (`swatch_base.stl` already exists in `frontend/public/`; verify it matches the STEP). No STEP/B-rep kernel at runtime.
2. Runtime: text → 2D outlines from TTF → extrude 0.6 mm (+ small overshoot above Z=3) → place in band → boolean subtract → binary STL.
3. Auto-shrink font if text width > 62 mm (current code just fails/overflows — check).
4. Kernel choice:
   - **First: `csgrs`** — pure Rust, has TTF text, extrude, booleans, STL I/O, builds for WASM.
   - **Fallback: `manifold-csg`** (manifold3d bindings) — more robust booleans, but needs cmake + C++ at build (hurts cross-compile / Android).
5. **Font:** replace bundled Arial (proprietary, cannot ship in a GPL repo) with **Liberation Sans** (SIL OFL, metric-compatible with Arial → same layout).
6. **Acceptance:** output is watertight/manifold; opens and slices in OrcaSlicer; dimensions/bands match the Python output for a fixture set of names (short, long, descenders, Æ/Ø/Å, digits); generation < 1 s on a Pi-class CPU.
7. **Bonus:** same crate compiled to WASM → live preview in the browser is the *real* mesh, not an approximation.

Removes: `swatch-service/`, Python/uv/Docker swatch options, `PYTHON_BIN`, `SWATCH_SERVICE_URL`, `publish-swatch.yml`.

**Done (session 33):** `crates/foreman-swatch` — csgrs boolean, base mesh = `swatch.step` tessellated at the Python exporter's tolerances (2,884 triangles; the old 27 MB `swatch_base.stl` measures the same within 0.009 % volume), Liberation Sans, auto-shrink to 62 mm (min 3 mm), output checked watertight before it is returned. None of the removed items existed in this repo (only in the old app). Details in doc 05, session 33.
