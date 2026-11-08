# Claude Code brief — Phase 1 close-out + Phase 2 (printers + live core, incl. Bambu LAN) + Phase 3 (Library, Send, queue, jobs)

Put this file in `docs/` next to 01–04. **Read first:** this file, then `docs/01-workflows-and-backend-map.md` (W-numbers below refer to it), then `docs/02-interaction-map.md`. The UI demo artifact (Foreman UI demo) is the reference for every screen; the API must serve what those screens show.

Same working rules as doc 04 (fish + Windows-safe commands, fmt/clippy/test before commit, no `unwrap()` in server paths, upstream wins, log corrections at the bottom). Tick boxes and commit after each session.

**Dev machine quirk (from the session 9–10 handoff):** prefix cargo with the explicit toolchain, e.g.
`RUSTC=~/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/rustc ~/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo test`

---

## Test hardware (owner's LAN)
| Printer | Firmware | IP | MAC | Use for |
|---|---|---|---|---|
| HellV0 (Voron V0) | Klipper + Moonraker | 10.0.0.177 | 4C:A3:8F:9D:0D:3D | C.2 Moonraker smoke, 2.3 Moonraker driver, 2.4 paths, 2.5 SSH/SFTP |
| Bambu Lab A1 | Bambu (LAN Only + Developer Mode) | 10.0.0.249 | 24:EC:4A:0C:07:90 | 2.3b Bambu LAN driver, FTPS, camera |

- Both IPs come from DHCP. Ask the owner to reserve them in the router, or the tests break. Put addresses in a gitignored `test-printers.local.toml`, never hard-coded.
- **HellV0 SSH (temporary, owner will change it):** user `biqu`, password `biqu`. Copy these into `test-printers.local.toml`. Don't put them in code, tests or commits. Once 2.5 works, install the Foreman ed25519 key on HellV0 and switch to key auth.
- Other secrets (A1 access code + serial, Moonraker API key if set) go only in that local file or env vars. Never commit them or write them to logs.
- A1 notes: it has no RTSPS camera (that's X1). Expect the P1/A1-style JPEG stream; verify the port and framing on this printer. AMS lite may be fitted; check what `report` shows.
- Never start a real print, move axes or heat anything without asking the owner first. Read-only checks (status, file list) are fine.

---

## Status at the start of this brief (sessions 1–10)

Done: 0.1–0.4, 1.1–1.4, 1.6, and the route-list part of 1.5 (51/51 v0.27.1 routes present).
Open: 0.5 Tauri, 0.6 CI, 1.5 compatibility gate (upstream suite, migration test, Moonraker smoke, load test), externaldb is a stub.

---

## Phase 1 close-out

### C.1 CI (was 0.6)
- [x] GitHub Actions: fmt, clippy `-D warnings`, test (Linux + Windows), frontend build, `cargo gen-types` diff check.
- [x] Weekly scheduled job: compare latest `Donkie/Spoolman` tag and the file list of `migrations/versions/` against v0.27.1; open an issue (or fail a non-blocking job) when they differ.

### C.2 Compatibility gate (was 1.5)
- [ ] `cargo compat` alias: starts DB containers (Docker **for tests only**), starts spoolman-rs, runs upstream `tests_integration/` with `SPOOLMAN_HOST`/`SPOOLMAN_PORT`. Green on SQLite, Postgres, MariaDB; Cockroach reported.
- [x] Migration test fixture: **owner has put an older real Spoolman DB in the local Foreman folder** — find it (ask if unclear), read its `alembic_version`, copy it to a gitignored `fixtures/` path (never commit it: personal data), start spoolman-rs on the copy, migrate, check row counts + sample spools unchanged, then open the migrated copy in upstream Python v0.27.1.
- [ ] Moonraker smoke test (containerised Moonraker, or HellV0 at 10.0.0.177 with its `[spoolman]` pointed at spoolman-rs temporarily; restore the original after): `spoolman_connected: true`, set/clear active spool, usage report lowers `used_weight`.
- [ ] Load test: 50 WebSocket clients + REST churn; record memory and p95 latency vs upstream in `docs/bench.md`.

### C.3 externaldb
- [ ] SpoolmanDB (external filament/material DB) download + local cache + refresh, matching upstream `externaldb` endpoints. Needed by the filament form's "Start from catalogue".

### C.4 Tauri shell (was 0.5) — can run in parallel with Phase 2
- [ ] Mode A (embed server) and Mode B (connect to URL) as in doc 04. Note WebKitGTK + Wayland + NVIDIA workarounds in README.

**Close-out done =** CI green, upstream suite green on 3 engines, Moonraker reports usage against spoolman-rs.

---

## Phase 2 — Printers + live core (W2, W3, W6)

### 2.1 Foreman DB + domain
- [ ] `foreman-core` SQLite DB (sqlx, own migrations, separate file from the Spoolman DB). Tables to start: `printer`, `printer_path` (connection tiers), `printer_ssh`, `webcam`, `macro_favourite`, `temp_preset`, `shell_macro`, `audit_log`, `attention_item`, `setting`.
- [ ] Domain types with `#[derive(TS)]` for every DTO the frontend uses.

### 2.2 Event bus
- [ ] Typed events on `/ws/events`: `printer.status`, `printer.connection` (path switch, offline), `attention.added/updated/cleared`, `job.*` (stub), `transfer.*` (Phase 3), `device.*` (Phase 4). Client can subscribe by topic and printer id. Keep heartbeat.

### 2.3 PrinterDriver trait + drivers
- [ ] Trait: connect, status stream, capabilities probe, commands (start/pause/resume/cancel, set temp, jog/home, macro, raw G-code, light), file ops (list/upload/delete/start) — file ops are used by Phase 3.
- [ ] **Moonraker:** WebSocket JSON-RPC with object subscriptions (not polling). Probe bed size, nozzle, extruders, MMU, webcams, macros from Moonraker/Klipper config.
- [ ] **OctoPrint** and **Duet:** polling drivers with per-driver interval.
- [ ] Mock printer servers (Moonraker + OctoPrint at least) that replay `test-gcodes/` scenarios; used in integration tests.

### 2.3b Bambu LAN driver (moved up from Phase 9 — owner has a real Bambu + a real Klipper printer to test on)
- [ ] MQTT over TLS, port 8883, user `bblp`, password = LAN access code, topics `device/<serial>/report` and `device/<serial>/request`. Send `pushall` on connect; merge partial reports into a full status (state, temps, fans, progress, layer, AMS trays with colour/material/remaining, HMS errors).
- [ ] Commands: pause/resume/stop, set temps, light, fan, G-code line, AMS tray select where supported. Map into the same `PrinterDriver` trait.
- [ ] FTPS (implicit TLS) on port 990 for file list/upload/delete on the SD card; start a print from an uploaded `.3mf` (project_file command). These feed the Library browser in Phase 3.
- [ ] Camera: X1 RTSPS stream; P1/A1 JPEG stream (verify ports/framing against a reference lib and the real printer; log what was found).
- [ ] Auth modes: **Developer Mode** first (printer in LAN Only mode, Developer Mode toggled on — no signing). **Signed mode** second: owner-supplied cert/key, commands signed; rejected commands return error 84033543 → raise an Attention item that says which mode is needed.
- [ ] Capabilities probe from model + report (bed size, nozzle, AMS count, chamber, camera type).
- [ ] Mock Bambu MQTT/FTPS server recorded from the owner's printer (capture real `report` payloads once, replay in tests).
- [ ] Use a reference implementation for message formats (e.g. OpenBambuAPI docs, ha-bambulab) — read, don't copy; note sources in the corrections log.

### 2.4 Tiered connection paths (W2)
- [ ] Each printer has an ordered list of paths (name + URL). Health check on the active path; after N failed checks (default 3, ≈6 s) switch to the next; move back up when a higher path is healthy for a set time (default 60 s, or manual). Running jobs are never touched.
- [ ] A switch emits `printer.connection` and raises an Attention item ("Using backup path: Tailscale").
- [ ] SSH, webcam proxy and uploads use the active path's host.
- [ ] API: CRUD + reorder + test-all for paths.

### 2.5 SSH / SFTP / terminal (W2)
- [ ] Per-printer SSH settings: user, port, auth = Foreman ed25519 key (generated once, public key shown for copy) or password (stored encrypted). Host key pinned on first connect (TOFU); refuse on change until approved in the UI.
- [ ] SFTP API (pick the crate — `russh` + `russh-sftp` were the plan; justify in the corrections log): list, read, write (writes a `.bak` first), upload, mkdir, rename, delete. Logs read-only.
- [ ] "Save + FIRMWARE_RESTART" for Klipper `.cfg`: deferred while printing.
- [ ] Terminal: WebSocket bridge to an SSH PTY (frontend uses xterm.js). Multi-line paste is confirmed in the UI before it is sent.
- [ ] Shell macros: name, multi-line command, scope (this printer / all SSH printers), ask-before-run (auto on for `sudo|rm|reboot|shutdown|dd|mkfs`). Stored in Foreman.
- [ ] Every terminal command and macro run goes to `audit_log` (printer, user, command, time).
- [ ] Bambu: none of this (no SSH). Its files go through FTPS (2.3b).

### 2.6 Printer page API (W6)
- [ ] Status (state, job, progress, ETA, layer), temperatures with **editable targets** + presets per printer, fan, light.
- [ ] Movement like Mainsail: home all / per axis, jog with step sizes, Z offset / babystep, motors off; disabled while printing except babystep.
- [ ] Macros: list from Klipper, favourites per printer; console with history (macros panel sits below console in the UI).
- [ ] Multiple webcams per printer (add/remove/order); proxy MJPEG/WebRTC streams.
- [ ] Cancel/stop always need confirmation in the UI (API just executes).

### 2.7 Fleet + Attention inbox (W3)
- [ ] Fleet list endpoint (cards: state, temps, progress, active spool(s), primary webcam, connection path).
- [ ] Attention items: print finished (clear plate), error/paused, runout, spool low, maintenance due, offline, path switched, failed upload. Each has actions (e.g. Plate cleared · start next, Rate outcome, Open printer). Actions return what changed so the UI can offer undo where reversible.

### 2.8 Settings → Printers
- [ ] Same printer settings as the printer page's Config tab, editable from Settings (inline per printer): name, auth (Moonraker key / Bambu access code / API key), tags, connection paths, SSH, hardened nozzle, sort order (drag order = Fleet order).

**Phase 2 done =** mock printers + the owner's real Klipper printer + real Bambu live on Fleet (temps, AMS, camera); killing the primary path fails over and back; SSH tab browses and edits `printer.cfg`; a file uploads to the Bambu over FTPS; Attention shows a finished print on both.

---

## Phase 3 — Library, Send, queue, jobs (W4, W5, W7)

### 3.1 File store + parser
- [ ] Library storage with folders (on disk + DB index), stars, trash with restore.
- [ ] Streaming G-code parser: slicer, printer model, nozzle, per-tool material + colour + grams/mm, time, layer count, **filament preset name** (`filament_settings_id`, verify per slicer: Orca, Prusa, Bambu Studio, Cura). Store per-tool data for compatibility + project colour slots.
- [ ] Thumbnails: embedded G-code thumbnails and 3MF `Metadata/plate_*.png`; STL → server-side render (pick an approach, keep it Rust; log the choice).
- [ ] Spool hint: when the preset name looks like the Orca Spoolman plugin's `Vendor Name (#ID) - Spoolman`, store spool id `#ID` as the file's suggested spool (used by Send in Phase 4).

### 3.2 Library browser API (one browser for everything)
- [ ] Locations: Library root + each printer's storage (Klipper over SFTP, Bambu over FTPS from 2.3b). List folder, search in folder, file details (preview, filament, time, grams, fits N printers).
- [ ] Copy / move between any two locations as a background **transfer** (server streams; printer→printer goes through Foreman). `transfer.*` events with progress.
- [ ] Conflict policy per request: keep both (` (copy)`), replace, skip. Never replace a file that is printing. Config files only to Klipper hosts. STL to a printer → warning "slice first".
- [ ] Delete files/folders: Library → trash (undo); printer → permanent. Skip anything printing.
- [ ] Upload, new folder, rename.

### 3.3 Compatibility engine (one Rust function)
- [ ] Input: file facts + printer capabilities + loaded spools. Output per printer: Ready / Busy / Plate (needs clearing) / Colour / Material / No spool / Too big / Nozzle, with reason text and an optional quick fix (e.g. "Plate cleared", "Load matching spool"). Used by Send, fleet queue, MCP.

### 3.4 Send
- [ ] Send endpoint: Print now / Queue on printer / Fleet queue; returns updated queue positions.
- [ ] OctoPrint-compatible upload endpoint for slicers (verify which routes OrcaSlicer/PrusaSlicer call: version + file upload with optional print), API-key protected; files land in Library "Slicer inbox".

### 3.5 Queues + scheduler (W5)
- [ ] Per-printer queues (reorder, remove, move between printers) + **fleet queue** ("next compatible free printer"; prefer printers with the right filament loaded).
- [ ] Scheduler reacts to events (idle, plate cleared, job added). Auto-start only after **plate cleared** confirmation.

### 3.6 Job engine (W7)
- [ ] Detect start/end/cancel/error per driver → job record → filament deduction → Attention item.
- [ ] `usage_reporter = moonraker | foreman` per printer: deduct exactly once (Moonraker `[spoolman]` reports itself).
- [ ] Outcome rating (good / failed + reason) from Attention or job history.
- [ ] Job history + basic stats endpoints (filters: printer, material, spool, outcome).

**Phase 3 done =** upload from OrcaSlicer → Library → Send → prints on a mock printer → job recorded, filament deducted once, Attention asks to clear the plate, next fleet-queue job starts after "Plate cleared".

---

## Session plan
| Session | Boxes |
|---|---|
| 11 | C.1 CI + upstream watch |
| 12 | C.2 compat harness + fix-until-green |
| 13 | C.2 migration test, Moonraker smoke, load test; C.3 externaldb |
| 14 | 2.1 Foreman DB + 2.2 event bus |
| 15 | 2.3 trait + Moonraker driver + mock |
| 16 | 2.3b Bambu LAN driver (MQTT + status + commands), tested on the real printer |
| 17 | 2.3b Bambu FTPS + camera + recorded mock; signed mode |
| 18 | 2.3 OctoPrint + Duet; 2.4 connection paths |
| 19 | 2.5 SSH / SFTP / terminal / shell macros / audit |
| 20 | 2.6 printer page API + 2.7 fleet + Attention + 2.8 settings |
| 21 | 3.1 file store + parser + thumbnails |
| 22 | 3.2 Library browser + transfers |
| 23 | 3.3 compatibility + 3.4 Send + slicer upload |
| 24 | 3.5 queues + scheduler |
| 25 | 3.6 job engine + history |
| any | C.4 Tauri shell when convenient |

Frontend for each box follows the UI demo; build screens as their APIs land (Fleet + Attention after 20, Printer page after 20, Library + Send after 23, Jobs after 25).

Bambu Cloud (Open Bamboo Networking helper) stays a later phase; see the oversight doc.

## Corrections log
(Claude Code: record any place where this brief disagreed with upstream source or reality, and what you did.)

### Session 13 — 2026-11-08 (migration test)

**DB found:** `spoolman-20251023-030101.db` at repo root. Revision `415a8f855e14` (second-to-last in chain; two migrations behind HEAD `fe4970567bb3`). Copied to `fixtures/old-spoolman.db` (gitignored). Added `fixtures/` and `test-printers.local.toml` to `.gitignore`.

**DB state before migration:** vendor=12, filament=1, spool=0, setting=3, filament_field=2, vendor_field=0, spool_field=0. No `tag` table (pre-tags revision). Single filament: Prusament Orange ASA with `url` and `hue_td` extra fields.

**Migration result:** `fe4970567bb3` (HEAD). All row counts unchanged. `tag` table added empty. Filament row byte-for-byte identical. Settings (including `extra_fields_filament` JSON) identical.

**Upstream Python v0.27.1 opens the migrated DB successfully:** Installed via `uv` into `/tmp/spoolman-pyenv` (Python 3.12). Needed: add venv `bin/` to `PATH` (alembic is called as a subprocess), set `PYTHONPATH` to vendor source + site-packages explicitly (uv venv uses symlinks so Python's venv auto-detection doesn't add site-packages). Create stub `vendor/spoolman-upstream/client_v2/build/index.html` to pass the client-build check. Upstream reports 1 filament, 12 vendors — all correct.

**Bug found and fixed: extra field values double-encoded on write, decoded on read (wrong per upstream contract).** Upstream contract (`spoolman/api/v1/models.py` line 57): *"Every value in the extra map is a JSON-encoded string regardless of the field's type. Consumers must JSON-decode each value."* spoolman-rs was calling `val.to_string()` on a `Value::String` in `set_values()`, which re-encoded an already-JSON-encoded string. On read, `get_values()` was JSON-decoding the stored value, returning native types instead of raw strings. Fix: `set_values()` stores `s.clone()` for `Value::String(s)`; `get_values()` / `get_values_batch()` / `get_distinct_values()` return `Value::String(raw_val)` without decoding. Two regression tests added (`extra_field_values_round_trip_as_raw_json_strings`, `extra_field_float_stored_by_upstream_survives_read`). All 13 tests pass.

### Session 11 — 2026-11-08 (C.1 CI)

**ci.yml already existed from session 1** with fmt, clippy, test (Linux + Windows), frontend build, and type-gen drift check — all correct. No changes needed to that file, only reformatting of previously committed Rust files that failed `cargo fmt --check` (`export.rs`, `main.rs`, `spool.rs`, `tag.rs`, `extra_field.rs`, `route_list.rs`). The fmt failures were pre-existing from sessions 9–10 but not caught before commit.

**Upstream-watch workflow** (`upstream-spoolman.yml`): uses `actions/github-script` to avoid shell JSON-escaping issues. Creates the `upstream-watch` label automatically if it doesn't exist. Opens or updates one issue when tag or migration file list diverges from `upstream-pin.json`. Non-blocking — warnings via `core.notice`, never fails CI.

**`upstream-pin.json`** pinned at `v0.27.1` with the 15 migration `.py` filenames. Excludes `__init__.py` and `__pycache__`.

**`.gitignore`**: added `fixtures/` and `test-printers.local.toml` (not present on `main`; had been added only on `migration-test` branch).
