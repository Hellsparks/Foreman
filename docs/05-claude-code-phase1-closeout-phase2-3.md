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
- **HellV0 SSH:** see `test-printers.local.toml` for credentials. Don't put them in code, tests or commits. Once 2.5 works, install the Foreman ed25519 key on HellV0 and switch to key auth.
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
- [~] `cargo compat` alias: starts DB containers (Docker **for tests only**), starts spoolman-rs, runs upstream `tests_integration/` with `SPOOLMAN_HOST`/`SPOOLMAN_PORT`. SQLite: 609 pass / 29 fail / 2 err (pre-existing: backup dedup, CORS, vendor fixture ordering). Postgres/MariaDB deferred — needs typed-bind redesign (`?`→`$N` via `tp()` done, but TIMESTAMP/BOOLEAN casts need per-engine bind types); WIP in stash `pg-wip`. Cockroach not yet attempted.
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
- [x] `foreman-core` SQLite DB (sqlx, own migrations, separate file from the Spoolman DB). Tables to start: `printer`, `printer_path` (connection tiers), `printer_ssh`, `webcam`, `macro_favourite`, `temp_preset`, `shell_macro`, `audit_log`, `attention_item`, `setting`.
- [x] Domain types with `#[derive(TS)]` for every DTO the frontend uses.

### 2.2 Event bus
- [x] Typed events on `/ws/events`: `printer.status`, `printer.connection` (path switch, offline), `attention.added/updated/cleared`, `job.*` (stub), `transfer.*` (Phase 3), `device.*` (Phase 4). Client can subscribe by topic and printer id. Keep heartbeat.

### 2.3 PrinterDriver trait + drivers
- [x] Trait: connect, status stream, capabilities probe, commands (start/pause/resume/cancel, set temp, jog/home, macro, raw G-code, light), file ops (list/upload/delete/start) — file ops are used by Phase 3.
- [x] **Moonraker:** WebSocket JSON-RPC with object subscriptions (not polling). Probe bed size, nozzle, extruders, MMU, webcams, macros from Moonraker/Klipper config.
- [ ] **OctoPrint** and **Duet:** moved post-v1 (no test hardware); backlog: OctoPrint, Duet, PrusaLink, Marlin USB — OctoPrint-compatible upload endpoint in 3.4 stays.
- [x] Mock printer servers (Moonraker + Bambu) that replay `test-gcodes/` scenarios; used in integration tests.

### 2.3b Bambu LAN driver (moved up from Phase 9 — owner has a real Bambu + a real Klipper printer to test on)
- [x] MQTT over TLS, port 8883, user `bblp`, password = LAN access code, topics `device/<serial>/report` and `device/<serial>/request`. Send `pushall` on connect; merge partial reports into a full status (state, temps, fans, progress, layer, AMS trays with colour/material/remaining, HMS errors).
- [ ] Commands: pause/resume/stop, set temps, light, fan, G-code line, AMS tray select where supported. Map into the same `PrinterDriver` trait.
- [~] FTPS (implicit TLS) on port 990 for file list/upload/delete on the SD card; start a print from an uploaded `.3mf` (project_file command). These feed the Library browser in Phase 3. **Code implemented (session 17) but port 990 is closed on the A1 — FTPS may be X1C/P1-only. Needs verification on other hardware.**
- [~] Camera: X1 RTSPS stream; P1/A1 JPEG stream (verify ports/framing against a reference lib and the real printer; log what was found). **Port 6000 is TCP-reachable on the A1 but TLS handshake times out (session 17, 3 attempts). Protocol may require active print or different framing.**
- [ ] Auth modes: **Developer Mode** first (printer in LAN Only mode, Developer Mode toggled on — no signing). **Signed mode** second: owner-supplied cert/key, commands signed; rejected commands return error 84033543 → raise an Attention item that says which mode is needed.
- [ ] Capabilities probe from model + report (bed size, nozzle, AMS count, chamber, camera type).
- [x] Mock Bambu MQTT/FTPS server recorded from the owner's printer (capture real `report` payloads once, replay in tests).
- [x] Use a reference implementation for message formats (e.g. OpenBambuAPI docs, ha-bambulab) — read, don't copy; note sources in the corrections log.

### 2.4 Tiered connection paths (W2)
- [x] Each printer has an ordered list of paths (name + URL). Health check on the active path; after N failed checks (default 3, ≈6 s) switch to the next; move back up when a higher path is healthy for a set time (default 60 s, or manual). Running jobs are never touched.
- [x] A switch emits `printer.connection` and raises an Attention item ("Using backup path: <label>").
- [ ] SSH, webcam proxy and uploads use the active path's host. (SSH done in 2.5; webcam/uploads deferred to later phase)
- [x] API: CRUD + reorder + test-all for paths.

### 2.5 SSH / SFTP / terminal (W2)

(Verified read-only on HellV0 in session 19: list config/gcodes/logs, open `printer.cfg`, terminal `uptime` + klipper status, audit entries. Writes are owner-tested.)
- [x] Per-printer SSH settings: user, port, auth = Foreman ed25519 key (generated once, public key shown for copy) or password (stored encrypted). Host key pinned on first connect (TOFU); refuse on change until approved in the UI.
- [x] SFTP API (pick the crate — `russh` + `russh-sftp` were the plan; justify in the corrections log): list, read, write (writes a `.bak` first), upload, mkdir, rename, delete. Logs read-only.
- [x] "Save + FIRMWARE_RESTART" for Klipper `.cfg`: deferred while printing.
- [x] Terminal: WebSocket bridge to an SSH PTY (frontend uses xterm.js). Multi-line paste is confirmed in the UI before it is sent.
- [x] Shell macros: name, multi-line command, scope (this printer / all SSH printers), ask-before-run (auto on for `sudo|rm|reboot|shutdown|dd|mkfs`). Stored in Foreman.
- [x] Every terminal command and macro run goes to `audit_log` (printer, user, command, time).
- [x] Bambu: none of this (no SSH). Its files go through FTPS (2.3b).
- [ ] Backlog: Live MJPEG webcam stream via proxy (currently 10 s snapshot).

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
- [x] Same printer settings as the printer page's Config tab, editable from Settings (inline per printer): name, auth (Moonraker key / Bambu access code / API key), connection paths, hardened nozzle, fail/return timing. (Tags, SSH, sort-order drag deferred to later.)

### 2.9 Connect screen + auth (moved up from Phase 6) — design in UI demo first; build before remote access.

**2.9a — auth (session 20a)**
- [x] Single admin account (Argon2id). Fresh server prints a one-time setup code (`XXXX-XXXX`, 24 h, single use) to the log; `POST /api/auth/setup` creates the admin. Min password 10.
- [x] Login → server-side sessions (hashed): browser HttpOnly SameSite=Strict cookie, other clients bearer token (`return_token`). Remember me 30 d, else 12 h. Logout. 5 failed tries per IP → 60 s pause.
- [x] API keys (create / list / revoke in Settings → Security, shown once, stored hashed, `X-Api-Key`).
- [x] Every `/api/*` and `/ws/*` route protected; open: `GET /api/health`, `GET /api/info`, `/api/auth/setup`, `/api/auth/login`.
- [x] Bind rule: no admin → 127.0.0.1 only (logged). Desktop: localhost unless "Let other devices connect" (needs admin, applies on restart).
- [x] `foreman-server reset-admin` CLI.
- [x] Spoolman API (7912) stays unauthenticated — logged at startup.
- [x] Frontend Setup + Sign-in screens (Connect.dc.html), sign-in on 401, Settings → Security (log out, API keys, LAN toggle). Audit user = admin username.
- [x] 2.9b — server picker, saved servers, mDNS, connection-lost view (session 20b):
  - [x] Stable `server_id` + editable server name (`PUT /api/settings/server`); `/api/info` adds `addresses[]` (LAN IPs, `hostname.local`, Tailscale IP + MagicDNS name when a 100.64.0.0/10 interface exists).
  - [x] mDNS announce `_foreman._tcp` (TXT id, name, version) only while bound to the LAN.
  - [x] Desktop Connect screens: welcome (saved servers, Another computer / This computer), connect (mDNS list, manual address, Test cases: Foreman needs sign-in / needs setup / Spoolman port / Moonraker printer / no answer, backups prefilled from `/api/info`), edit (reorder, test, remove, Test and add with same-id check, Forget), "Add as backup route" for a known server id.
  - [x] Saved servers + bearer tokens in the app data folder (`servers.json`, 0600), not localStorage.
  - [x] Failover: 3 failed checks → next address with the same token (server id must match), "via …" tag in the title bar, back to the primary after 60 s healthy, lost-connection panel only when all addresses are down.
  - [x] Mode A ("This computer") starts the embedded server on demand; "Let other devices connect" goes through setup with the code prefilled. Browser keeps login only.
**Phase 2 done =** mock printers + the owner's real Klipper printer + real Bambu live on Fleet (temps, AMS, camera); killing the primary path fails over and back; SSH tab browses and edits `printer.cfg`; a file uploads to the Bambu over FTPS; Attention shows a finished print on both.

---

## Phase 3 — Library, Send, queue, jobs (W4, W5, W7)

### 3.1 File store + parser
- [x] Library storage with folders (on disk + DB index), stars, trash with restore.
- [x] Streaming G-code parser: slicer, printer model, nozzle, per-tool material + colour + grams/mm, time, layer count, **filament preset name** (`filament_settings_id`, verify per slicer: Orca, Prusa, Bambu Studio, Cura). Store per-tool data for compatibility + project colour slots.
- [x] Thumbnails: embedded G-code thumbnails and 3MF `Metadata/plate_*.png`; STL → server-side render (pick an approach, keep it Rust; log the choice).
- [x] Spool hint: when the preset name looks like the Orca Spoolman plugin's `Vendor Name (#ID) - Spoolman`, store spool id `#ID` as the file's suggested spool (used by Send in Phase 4).

### 3.2 Library browser API (one browser for everything)
- [x] Locations: Library root + each printer's storage (Klipper over SFTP, Bambu over FTPS from 2.3b). List folder, search in folder, file details (preview, filament, time, grams, fits N printers). *(Klipper via Moonraker's file API, Bambu parked — see session 21/22 log; "fits N printers" waits for 3.3.)*
- [x] Copy / move between any two locations as a background **transfer** (server streams; printer→printer goes through Foreman). `transfer.*` events with progress.
- [x] Conflict policy per request: keep both (` (copy)`), replace, skip. Never replace a file that is printing. Config files only to Klipper hosts. STL to a printer → warning "slice first". *(STL is refused; config files are not in the Library.)*
- [x] Delete files/folders: Library → trash (undo); printer → permanent. Skip anything printing.
- [x] Upload, new folder, rename.

### 3.3 Compatibility engine (one Rust function)
- [ ] Input: file facts + printer capabilities + loaded spools. Output per printer: Ready / Busy / Plate (needs clearing) / Colour / Material / No spool / Too big / Nozzle, with reason text and an optional quick fix (e.g. "Plate cleared", "Load matching spool"). Used by Send, fleet queue, MCP.

### 3.4 Send
- [ ] Send endpoint: Print now / Queue on printer / Fleet queue; returns updated queue positions.
- [x] OctoPrint-compatible upload endpoint for slicers (verify which routes OrcaSlicer/PrusaSlicer call: version + file upload with optional print), API-key protected; files land in Library "Slicer inbox".

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
| 18 | 2.4 connection paths |
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

### Session 22 — 2026-11-10 (3.2 Library browser + transfers + Library UI)

- **Step 0.** (a) The listener is now dual-stack: `0.0.0.0`/`::` binds one `[::]` socket with `IPV6_V6ONLY=false` (`socket2`), and loopback binds `127.0.0.1` plus `[::1]` (best effort), so `localhost` works whichever family a client such as flatpak Orca tries first. IPv4-mapped peers (`::ffff:a.b.c.d`) are unwrapped before rate limiting and logging. (b) Every 401/403 is logged at `warn` with IP, method, route (path only, never the query, so no URL token) and a reason (`no credentials`, `invalid API key`, `invalid or expired session`, `invalid or expired URL token`, `server not set up`, or the handler's code such as `bad_credentials`). At most one line per IP + reason per minute; the next line reports how many were suppressed. No key, token or password is ever logged. (c) The STL renderer already parsed with `stl_io` 0.11; nothing changed.
- **Moonraker file API**, verified against moonraker.readthedocs.io (`external_api/file_manager`) and the real HellV0: `GET /server/files/directory?path=gcodes/<dir>&extended=true` (metadata, with `thumbnails[].relative_path` relative to the file's folder), multipart `POST /server/files/upload` (`root`, `path`, `file`; Moonraker creates missing folders and spools to a temp file, so a cancelled upload leaves nothing), `GET`/`DELETE /server/files/gcodes/<file>`, `POST`/`DELETE /server/files/directory` (`force=true`), `POST /server/files/move` and `/copy` (both overwrite an existing file). Responses are wrapped in `{"result": …}`. `X-Api-Key` is sent when the printer has an API key. The printing-file guard asks the printer directly (`GET /printer/objects/query?print_stats=state,filename`; `printing` or `paused` counts) instead of trusting the cached status. Implemented as `MoonrakerFiles` (`PrinterStorage` trait); the driver exposes it through `PrinterDriver::storage()`, and `list_files`/`upload_file`/`delete_file` now work too. `start_print` is still not implemented (Send is next).
- **Transfers** run in memory (not persisted across restarts), 2 at a time, with `transfer.started` / `transfer.progress` (every 300 ms) / `transfer.finished` events carrying a `TransferInfo`. Printer→printer downloads to a Foreman temp file and uploads, so its byte total counts both legs. Moves on the same location are server-side (Library rename, Moonraker move/copy). A cross-location move deletes the source only after the copy succeeded: Library sources go to the trash, printer sources are deleted. Cancel aborts the task, and the temp file is removed on drop.
- **Conflicts**: `POST /api/transfers` with `on_conflict: "ask"` returns **409** with the clashes (`index`, `name`, `dest_printing`); the client resends with a per-item `on_conflict` (keep_both / replace / skip), and the UI's "Apply to all" fills it for the rest. Keep both = ` (copy)`, then ` (copy 2)`, … Replacing a Library file moves the old one to the trash (restorable). Replace is refused for a printing file, for a file↔folder clash, and for a folder on the same location ("pick Keep both"); across locations, replacing a folder merges into it. Moving into the folder it is already in is skipped. STL to a printer is refused ("slice first"); inside a folder copy, STLs are skipped and the transfer's `note` says how many.
- **REST added**: `GET /api/locations`; `GET /api/printers/{id}/storage?path=`, `/storage/thumb`, `/storage/download` (both URL-token aware), `POST /storage/folders|rename|delete|upload`; `GET|POST /api/transfers`, `POST /api/transfers/{id}/cancel`, `POST /api/transfers/clear`; `POST /api/library/trash-many` and `/restore-many` (multi-select delete + undo). Driver errors map to 502. `LibraryEntry` gained `print_time_secs` and `tools` so grid cards show material and grams without a request per file.
- **Library UI** follows `Library.dc.html` (flat `.fw-browser` only): sidebar locations (All files, top-level Library folders, Trash, Klipper printers with a state dot), breadcrumbs with a location note, grid cards with thumbnails, multi-select (tick, Ctrl/Cmd/Shift-click, Ctrl/Cmd-A, Esc), copy/cut/paste from the toolbar and keyboard, the carried-selection banner (Copy N here / Move here), drag onto folders and sidebar locations (same location = move, else copy), the conflict bar, the transfers list with progress and Cancel, a Trash view with Restore, and the inspector (preview, filament, time, grams, colours, slicer, printer profile, filament preset, nozzle, changed; Download, Rename, Star, Delete with a second-click confirm). The undo for a Library delete sits in the browser footer status, as in the reference, rather than in a shell toast. Send is a disabled "Send (next session)" button; the FITS fact waits for 3.3.
- **Real printer (HellV0)**: listed gcodes (45 files, idle), copied `plugg rune.gcode` HellV0 → Library (parsed, thumbnail), created `foreman-test/`, copied Library → `HellV0/foreman-test` and back (conflict → Keep both), trashed and undid in the Library, uploaded and deleted a 4-byte test file inside `foreman-test/`, and saw an STL upload refused. Nothing outside `foreman-test/` was written or deleted; `foreman-test/plugg rune.gcode` was left in place.

### Session 21 — 2026-11-10 (3.1 file store + parser + thumbnails, 3.4 slicer upload)

- **Plan change (owner) — Library locations = Library + Klipper printers only.** Bambu storage is parked: the A1 runs in cloud mode and FTPS :990 is closed (see 2.3b). For 3.2, Klipper G-code goes through **Moonraker's file API** (`/server/files/*`); SSH/SFTP stays for config files only. The 3.4 slicer upload moved forward from session 23 to 21.
- **Store layout:** files live in `<data_dir>/library`, with `library-trash/<trash_id>/` for trash and `library-cache/{thumbs,tmp}` for cache. DB tables `library_entry` (path, parent, kind, size, sha256, mtime, added_at, starred, deleted_at, trash_id, trash_root) and `file_meta` (migration 0007). On startup the index is rebuilt against the disk, and the disk wins: new files are added and parsed, files with a changed size or mtime are re-hashed and re-parsed, and missing rows are dropped. Deleting a folder trashes the whole subtree as one group. Only the root can be restored; if its name was taken meanwhile it gets a ` (n)` suffix. A daily purge deletes trash older than `library.trash_days` (default 30, range 1–365, `GET/PUT /api/settings/library`). Uploads with a name already in use are kept as ` (n)` (OctoPrint overwrites; Foreman never replaces silently).
- **Parser:** reads the first 1 MiB and last 512 KiB of each file, so the cost is the same for any size. Keys were verified against real files: **OrcaSlicer 2.3.0** (config block at the end, `; total layer number`, `printer_settings_id`, `printer_model`, `filament used [g]/[mm]`), **PrusaSlicer 2.9.7** (`.gcode` + `.bgcode`), and **Bambu Studio 02.08** (plate G-code with the config block at the start, plus a real `slice_info.config`). **Cura** uses a synthetic fixture built from the documented `;FLAVOR/;TIME/;Filament used/;TARGET_MACHINE.NAME` header, best effort, because no Cura install was available. Fixtures are trimmed headers and footers only. Bambu Studio's CLI could not export a 3MF on this machine (GL error), so the 3MF test fixture is built inside the test from the real `slice_info.config` and the real plate G-code.
- **Owner addition:** `printer_settings_id` (printer profile name) and, for Bambu 3MF, `printer_model_id` from `slice_info.config` (e.g. `N2S` = A1) are stored in `file_meta` for the 3.3 compatibility engine. Per-plate time, grams and filaments are stored as `plates_json`.
- **bgcode is supported, not parked:** the pure-Rust crate `binarygcode` 0.0.3 (a port of libbgcode) reads the file/printer/slicer metadata blocks and PNG thumbnails, and stops at the first G-code block. QOI/JPG thumbnails are skipped.
- **3MF:** read with `zip` 9 + `quick-xml` 0.42. Meta comes from `Metadata/slice_info.config` plus the first `Metadata/plate_N.gcode`, else `project_settings.config`. Thumbnail order is `plate_N.png`, then `Metadata/thumbnail.png`, then `Auxiliaries/.thumbnails`. An unsliced 3MF still lists, with the slicer taken from `3D/3dmodel.model`.
- **STL render — crate choice:** there is no GPU-free pure-Rust STL thumbnailer, so Foreman has a small software rasteriser (`stl_io` 0.11 + `png` 0.18): isometric view, z-buffer, flat two-sided shading, 256 px with a transparent background. It worked on the first attempt, so there is no backlog line.
- **Thumbnails** are cached as PNG under `library-cache/thumbs/<id>.png` and served at `GET /api/library/files/{id}/thumb?v=<hash12>`. `/thumb` and `/download` were added to the URL-token allow-list.
- **OctoPrint upload, verified** against docs.octoprint.org (`/api/version`, `/api/files`) and against OrcaSlicer's client source (`src/slic3r/Utils/OctoPrint.cpp`, which PrusaSlicer shares):
  - "Test" calls `GET /api/version` with `X-Api-Key` and requires an `api` key and a `text` value starting with `OctoPrint`. Foreman answers `OctoPrint 1.10.3 (Foreman x.y.z)`.
  - Upload is `POST /api/files/local` as multipart with `print`, `path`, optional `plateindex` (Orca/Bambu), and `file`. Foreman returns 201 with a `Location` header and `{done, effectivePrint:false, effectiveSelect:false, files.local{name, path, origin, refs}}`.
  - Files land in `Slicer inbox/<path>`. `print=true` is accepted and only logs "Send not built yet". Auth uses a normal Foreman API key. `GET /api/server` is also present.
  - Body limit for the upload routes is 2 GiB.
- **Library REST:** `GET /api/library?path=`, `GET /api/library/trash`, `GET /api/library/files/{id}` (entry + meta), `/thumb`, `/download`, `POST /api/library/upload?path=` (multipart), `POST /api/library/folders`, `PATCH /api/library/entries/{id}` `{name?, starred?}`, `DELETE /api/library/entries/{id}` (to trash), and `POST /api/library/entries/{id}/restore`.

### Session 20c — 2026-11-10 (Phase 2 finish: unclaimed servers on the LAN, scoped URL tokens)

- **Decision — unclaimed servers listen on the LAN** (replaces the 2.9a "no admin → 127.0.0.1 only" rule): a headless server binds its configured host before setup, but answers only `GET /api/health`, `GET /api/info` and `POST /api/auth/setup`; every other `/api/*` and `/ws/*` route returns **403 `{code: "setup_required", error: "server not set up"}`** (login included). Setup shares the per-IP limiter: 5 wrong codes → 60 s pause (429 + `Retry-After`). mDNS TXT gains `needs_setup=1|0` and is re-announced right after setup, so the Connect list shows "Not set up" for a fresh Pi. The static frontend is still served (it holds no data) so a browser can reach the setup page. Exceptions: desktop Mode A stays loopback-only until an admin exists and "Let other devices connect" is on, and its in-process app session still works before setup (it can't be obtained over the network). Bind-rule unit test and router tests updated.
- **Decision — scoped URL tokens replace session tokens in URLs** (replaces the 20b `?token=<session>` note): `POST /api/auth/url-token {path}` (any authenticated caller) returns a `fmu_…` token valid **60 s for that exact path only**, allowed for `/api/printers/{id}/snapshot`, `/api/printers/{id}/files/download`, `/ws/events` and `/ws/printers/{id}/terminal`. Grants live in memory, stored hashed, purged on issue. Session tokens in a URL are refused everywhere. The desktop app fetches URL tokens on demand (reused for 45 s, refreshed every 40 s for open download links); the browser keeps using its cookie. Request spans log the URI with `token=REDACTED` (verified with `RUST_LOG=foreman_server=debug,tower_http=debug`).

### Session 20b — 2026-11-09 (2.9b connect screen + Mode B)

- **Branch base:** `phase2-connect` is stacked on `phase2-auth` (PR #3 not merged yet); its PR shows the auth commits until #3 lands.
- **Listener moves without a restart** (replaces the 20a "applies on restart" note): the bind host is a `watch` value; creating the admin, the LAN toggle and a rename re-evaluate the bind rule and re-bind the listener in place (drivers and open connections untouched). A headless server configured for `0.0.0.0` therefore goes on the LAN right after setup. The default `host` is still `127.0.0.1`, so a headless server needs `FOREMAN__HOST=0.0.0.0` to be reachable/announced.
- **mDNS** via `mdns-sd`, announced only when not bound to loopback, re-announced on rename. Container/VM bridges (`docker*`, `br-*`, `veth*`, `virbr*`, …) are left out of the announced and reported addresses. MagicDNS name comes from `tailscale status --json` at startup (skipped when the CLI is missing).
- **`/api/info` `addresses`** are objects `{ address: "host:port", kind: lan_ip | lan_name | tailscale_ip | tailscale_name | loopback }` so the app can label routes.
- **Desktop calls the server directly** (CORS is already permissive) with `Authorization: Bearer`. `<img src>`/`<a href>` and WebSockets can't send headers, so `?token=` is accepted on `/ws/*` and on `GET /api/printers/{id}/snapshot` and `/files/download` only (REST still refuses it).
- **Probing and health checks run in Rust** (Tauri commands `probe_address`, `check_address`, `discover_servers`) to avoid CORS/mixed-content limits on non-Foreman hosts. A bare address tries `:9090`; when that fails it also looks for Moonraker on `:7125` and `:80`. `https://` is kept when typed; no automatic https upgrade yet (the server has no TLS).
- **Mode A sessions:** the app mints a session in-process (`LocalHandle::app_token`), so "This computer" works without a password while localhost-only (audit user `local`). The embedded server now starts only when Mode A is chosen, and keeps running if the user switches to a remote server (no in-process shutdown of drivers).
- **Unreachable backups:** Edit → "Test and add" refuses a different server id; a silent address can still be added with "Add anyway" (as the reference text suggests) and is only used once it answers with the right id.
- **Frontend tests:** added `vitest` (failover + address helpers); `npm test` now runs them.

### Session 20a — 2026-11-09 (2.9a auth)

- **Token hashing:** session tokens (`fms_…`) and API keys (`fmk_…`) are 256-bit random values, stored as SHA-256 hex. Argon2id is only for the admin password — a slow KDF adds nothing for full-entropy tokens and would make every request expensive.
- **Setup code lives in memory** (not the DB): restarting the server prints a new one and the old one dies. Matches the Connect.dc.html hint text. Wrong codes count toward the per-IP rate limit.
- **WebSocket auth:** browsers send the cookie on the upgrade; non-browser clients that can't set headers may pass `?token=` on `/ws/*` only (never on REST).
- **No `Secure` cookie flag:** the server speaks plain HTTP on the LAN today; `Secure` would break sign-in. Revisit with TLS / remote access. CSRF is covered by `SameSite=Strict`.
- **Tauri uses the cookie, not a bearer token:** the webview loads the frontend same-origin (Vite proxy in dev), so the HttpOnly cookie works and no token has to be kept in JS-readable storage. The bearer path (`return_token: true` on login/setup, `Authorization: Bearer`) is implemented and tested for other clients; the desktop app switches to it in 2.9b when it talks to a remote server.
- **API keys can't manage auth:** an `X-Api-Key` request can use the API but can't list/create/revoke keys or change the network setting (403) — a leaked slicer key can't mint more keys.
- **"Let other devices connect"** is stored as setting `network.allow_lan` and applies on next start (the listener is bound once). Settings shows "Restart Foreman to apply". Likewise a headless server set to `0.0.0.0` keeps 127.0.0.1 until restarted after setup (logged).
- **Spoolman API (7912) stays open** as instructed; a startup log line says so.
- **`GET /api/info`** also gets `server_id` (random UUID, setting `server.id`) and `name` (setting or hostname) for the 2.9b server picker.
- **Step 0 clean-up:** `openprinttag.rs` hand-rolled SHA-1 replaced by `Uuid::new_v5` (new test pins Python `uuid.uuid5` reference values, passed before and after); `api/tag.rs` local hex module replaced by the `hex` crate.
### Session 19 — 2026-11-09 (2.5 SSH / SFTP / terminal / shell macros)

**Crate choice:** `russh` 0.65 + `russh-sftp` 2.4 (as planned). Pure Rust, tokio-native, no libssh2/OpenSSL system dependency (`ssh2` would need libssh2 + OpenSSL on Windows CI). Gives client PTY/shell/exec/subsystem channels, and its server side lets the TOFU test run against a real in-process SSH server (`crates/foreman-core/tests/ssh_pinning.rs`). `ssh-key` is pinned to `=0.7.0-rc.11` (the version russh 0.65 uses) with the `rand_core` feature for ed25519 keygen.

**Encryption at rest:** brief said "stored encrypted" without a key source. A random 256-bit master key is created once in `<data_dir>/secret.key` (0600 on Unix), outside the DB; the Foreman private key (`setting.ssh.private_key`) and SSH passwords (`printer_ssh.password`) are AES-256-GCM sealed (`enc:v1:` prefix). A copied DB alone reveals nothing; DB + `secret.key` together do (no OS keychain yet — same machine trust model as the DB). Losing `secret.key` means re-entering passwords and re-installing a new Foreman public key.

**Host-key change flow:** a changed key is refused, stored in a new column `printer_ssh.pending_host_key_fingerprint` (migration 0004), and raises Attention kind `ssh_host_key`. "Trust new host key" (`POST /ssh/trust`) pins exactly the pending fingerprint the server observed — the client can't submit an arbitrary fingerprint. Fingerprint format = OpenSSH `SHA256:…`; verified identical to `ssh-keyscan | ssh-keygen -lf -` on HellV0.

**Quick roots:** resolved per connection from SFTP `realpath(".")` (the SSH user's home) + `printer_data/{config,gcodes,logs}`. Read-only logs is enforced server-side on the normalised path *and* the realpath of the parent (so a symlink into logs can't be written through). Large files (>2 MB) are read as a read-only tail; Download streams the full file.

**Save + FIRMWARE_RESTART:** the brief says "deferred while printing"; owner's session brief says "refused while printing (server-side)". Implemented as refuse: while printing/paused (or state unknown/offline) the request is rejected with 400 and nothing is written — plain Save still works. The state is re-checked after saving and before sending `FIRMWARE_RESTART` (as G-code via the Moonraker driver). Only `.cfg` under `printer_data/config` qualifies.

**Terminal / macros:** PTY shell over one SSH session per WebSocket (`/ws/printers/{id}/terminal`), xterm.js + fit addon in the UI. Macros run *in the terminal* (`{"type":"macro","id":N}`), so output is visible and the server (not the client) types the stored lines; the old exec-per-command REST runner and the server-side command deny-list from the earlier WIP were dropped (the brief asks for ask-before-run, not blocking). Ask-before-run is forced on server-side when the body matches `\b(sudo|rm|reboot|shutdown|dd|mkfs)\b`. Audit of typed commands is best effort: lines are rebuilt from keystrokes (backspace, Ctrl-C/U, escape sequences handled; tab-completion / history recall can't be seen). SFTP write/upload/mkdir/rename/delete are also audited; `audit_log.user` = SSH user (no Foreman accounts yet).

**UI colours:** the reference terminal/editor uses fixed dark hex colours. Implemented with Fieldwork tokens only (`--fw-surface` / `--fw-text` / `--fw-accent`, read at runtime into the xterm theme and updated on theme/mode change). Built-in quick buttons from the reference (uptime, df -h /, klipper status, tail klippy.log) are kept as non-editable; `systemctl status` gets `--no-pager` for a PTY.

**Upload:** raw request body streamed to SFTP (`POST /files/upload?path=`), not multipart; overwriting an existing file keeps a `.bak` like Save.

**Connection lifetime (owner request):** SFTP calls reuse one pooled SSH session per printer (first call ~3 s on HellV0, then ~30 ms). The terminal and Files tabs stay mounted after first open, so switching tabs no longer drops the shell. Both have a "Close connection" button; idle auto-close is per printer (`printer_ssh.idle_timeout_secs`, migration 0005, default 15 min, 0 = never), enforced server-side for pooled sessions (30 s reaper) and terminals (no input/output). Changing SSH settings, trusting a new key or removing the printer closes the pooled session.

**HellV0 verification:** handshake + TOFU pin verified on the real printer (fingerprint matches ssh-keyscan). `test-printers.local.toml` has no HellV0 SSH credentials, so listing/reading `printer.cfg` and running `uptime` in the terminal are still to be verified — boxes marked `[~]` until then.

### Session 18 — 2026-11-09 (2.4 connection paths + Config tab)

**Health-check protocol:** Brief said "driver manager health-checks the active path." Implemented as TCP connect (not HTTP GET) to the host+port parsed from the path URL. This is protocol-agnostic — works for Moonraker (`http://host:7125` → port 7125), Bambu bare IPs, and mDNS hostnames. `reqwest` was considered but dropped in favour of `tokio::net::TcpStream` to avoid HTTP semantics.

**`primary_url` overload during path switch:** `switch_to_path` temporarily overrides `printer.primary_url` with the new path's URL before calling `start_driver`, which reads `primary_url` to build the driver config. The DB row is not updated — only the in-memory `Printer` passed to `start_driver`. The DB `primary_url` column is now effectively a "default if no paths defined" fallback.

**SSH/webcam path following (item 3 of 2.4):** Deferred — SSH is 2.5 (not yet built); webcam proxy and uploads are later phases. The active path URL is exposed via `GET /api/printers/:id/active-path` so future implementors can consume it.

**2.8 scope:** Implemented core Config form in both Printer page (Config tab) and Settings page (expandable per-printer form). Tags and sort-order drag were deferred — tags need a tag input component not yet built; drag-to-reorder needs a DnD library choice decision.

### Session 15 — 2026-11-08 (PrinterDriver + Moonraker + Fleet UI)

**Moonraker API version:** HellV0 reports Moonraker v0.10.0-9 (API 1.5.0), newer than the v0.27.1 Spoolman doc's assumed version. No incompatibilities found — JSON-RPC methods (`server.info`, `printer.objects.subscribe`, `printer.objects.query`, `server.webcams.list`, `printer.objects.list`) work identically.

**API key delivery:** Brief mentions `X-Api-Key` header for Moonraker auth. Moonraker's WebSocket actually accepts API keys as a `?token=<key>` query parameter on the WS URL, not as a header. Changed to query param approach since `tokio-tungstenite::connect_async` with a custom `http::Request` caused `HandshakeIncomplete` errors in tests.

**HellV0 capabilities probe:** bed 120x120mm, 1 extruder, 0 webcams (none configured in moonraker.conf), 0 public macros (all macros on this printer start with `_`). No MMU detected.

**Tauri CLI not installed** on this dev machine — could not test `tauri dev`. Verified the full data path via headless server + curl: HellV0 added via REST, live temps (nozzle ~26°C, bed ~26°C) returned in `GET /api/printers` and via WebSocket events. Frontend builds clean and renders the Fleet page.

### Session 16 — 2026-11-09 (Bambu LAN driver — MQTT + status)

**Reference sources for Bambu MQTT protocol (read, not copied):**
- OpenBambuAPI: `github.com/Doridian/OpenBambuAPI/blob/main/mqtt.md` — MQTT topic structure, pushall command format, report field names and types, AMS tray schema, HMS code format.
- Bambu wiki: `wiki.bambulab.com/en/knowledge-sharing/developer-resources/bambu-mqtt-overview` — official LAN MQTT overview.
- ha-bambulab: `github.com/greghesp/ha-bambulab` — Home Assistant integration, validated field mappings for A1/P1/X1 series (especially fan speed 0–15 → percent mapping, serial prefix → model mapping, AMS Lite detection).

**Fan speed encoding:** Bambu reports fan speeds as string values "0"–"15" (not percentages). Driver maps `raw / 15 * 100` to get 0–100%. Confirmed against ha-bambulab and real A1 report observations from other users.

**Serial prefix → model mapping:** `00W` = A1, `01S` = A1 Mini, `00M` = X1C, `01P` = P1P, `03P` = P1S, `030` = X1E. These are derived from community observations (OpenBambuAPI, ha-bambulab) — Bambu doesn't publish an official model-to-serial mapping in MQTT. The `dev_model_name` field is only in the cloud API.

**TLS:** Bambu printers use a self-signed certificate. The driver uses rustls with a custom `ServerCertVerifier` that accepts any cert (TOFU approach). Certificate fingerprint pinning (store on first connect, refuse on mismatch) is deferred — requires DB schema for pinned certs.

**MQTT crate:** chose `rumqttc v0.24` with `use-rustls` feature. Initially tried `use-native-tls` but rumqttc's native-tls backend doesn't expose `danger_accept_invalid_certs` on the `TlsConfiguration` enum — only `Native` (default OS certs) and `SimpleNative` (custom CA PEM) are available. Switched to rustls which allows injecting a custom `ClientConfig` with a custom cert verifier.

**Commands:** All `PrinterDriver` trait command methods return `NotImplemented` this session per the read-only constraint. Wiring actual MQTT commands (pause, resume, set temp, etc.) is deferred.

### Session 13 — 2026-11-08 (migration test)

**DB found:** `spoolman-20251023-030101.db` at repo root. Revision `415a8f855e14` (second-to-last in chain; two migrations behind HEAD `fe4970567bb3`). Copied to `fixtures/old-spoolman.db` (gitignored). Added `fixtures/` and `test-printers.local.toml` to `.gitignore`.

**DB state before migration:** vendor=12, filament=1, spool=0, setting=3, filament_field=2, vendor_field=0, spool_field=0. No `tag` table (pre-tags revision). Single filament: Prusament Orange ASA with `url` and `hue_td` extra fields.

**Migration result:** `fe4970567bb3` (HEAD). All row counts unchanged. `tag` table added empty. Filament row byte-for-byte identical. Settings (including `extra_fields_filament` JSON) identical.

**Upstream Python v0.27.1 opens the migrated DB successfully:** Installed via `uv` into `/tmp/spoolman-pyenv` (Python 3.12). Needed: add venv `bin/` to `PATH` (alembic is called as a subprocess), set `PYTHONPATH` to vendor source + site-packages explicitly (uv venv uses symlinks so Python's venv auto-detection doesn't add site-packages). Create stub `vendor/spoolman-upstream/client_v2/build/index.html` to pass the client-build check. Upstream reports 1 filament, 12 vendors — all correct.

**Bug found and fixed: extra field values double-encoded on write, decoded on read (wrong per upstream contract).** Upstream contract (`spoolman/api/v1/models.py` line 57): *"Every value in the extra map is a JSON-encoded string regardless of the field's type. Consumers must JSON-decode each value."* spoolman-rs was calling `val.to_string()` on a `Value::String` in `set_values()`, which re-encoded an already-JSON-encoded string. On read, `get_values()` was JSON-decoding the stored value, returning native types instead of raw strings. Fix: `set_values()` stores `s.clone()` for `Value::String(s)`; `get_values()` / `get_values_batch()` / `get_distinct_values()` return `Value::String(raw_val)` without decoding. Two regression tests added (`extra_field_values_round_trip_as_raw_json_strings`, `extra_field_float_stored_by_upstream_survives_read`). All 13 tests pass.

### Session 17 — 2026-11-09 (Bambu real A1 verify + TOFU pinning + FTPS/camera)

**Real A1 capture:** Connected to A1 at 10.0.0.249:8883 via MQTT/TLS, captured 43 messages, stripped serial/access_code/IP, wrote 6-message fixture (`bambu_a1_reports.json`). Replay test verifies state (FAILED), temps (25.09°C nozzle, 24.69°C bed), 5 AMS trays (4 AMS Lite + vt_tray slot 254), real colours, fans off, HMS empty.

**Serial prefix `039`:** Real A1 serial starts with `039` (not `00W`). Added mapping in `model_from_serial`. Confirmed via ha-bambulab community data — newer A1 production batches use `039`.

**TOFU cert pinning (replacing AcceptAnyCert):** Implemented `TofuCertVerifier` using rustls `ServerCertVerifier` trait. SHA-256 fingerprint captured during TLS handshake via `Arc<Mutex<Option<String>>>`. First connect stores fingerprint in DB (`printer.tls_fingerprint` column, migration 0002). Subsequent connects verify match; mismatch → refuse + publish `printer.connection` error event. API endpoint `POST /api/printers/{id}/trust-new-cert` clears the pin and restarts the driver.

**FTPS port 990 closed on A1:** Port scan shows only 3000, 6000, 8883 open. FTPS code implemented (`list_files`, `upload_file`, `delete_file` via sync `NativeTlsFtpStream` in `spawn_blocking`) but untestable on this printer. A1 may not expose FTPS — FTPS is documented mainly for X1C/P1 series.

**Camera port 6000 TLS timeout:** Port 6000 is TCP-reachable (SYN-ACK within 1ms) but all TLS handshake attempts time out (3 attempts: raw TLS wrap, auth-frame-before-TLS, OpenBambuAPI 68-byte auth packet with 0x40 0x3d magic). The A1 camera protocol may differ from documented X1 protocol or may only activate during a print.

**Port 3000:** Responds to TCP connections, contents unknown. Not documented in OpenBambuAPI.

**DB migration approach:** Initial schema `0001_initial.sql` now includes `tls_fingerprint` column directly. `0002_tls_fingerprint.sql` kept as incremental `ALTER TABLE` for existing dev databases. `ForemanDb::run_migrations()` applies these after schema creation, ignoring "duplicate column" errors for idempotency.

### Session 11 — 2026-11-08 (C.1 CI)

**ci.yml already existed from session 1** with fmt, clippy, test (Linux + Windows), frontend build, and type-gen drift check — all correct. No changes needed to that file, only reformatting of previously committed Rust files that failed `cargo fmt --check` (`export.rs`, `main.rs`, `spool.rs`, `tag.rs`, `extra_field.rs`, `route_list.rs`). The fmt failures were pre-existing from sessions 9–10 but not caught before commit.

**Upstream-watch workflow** (`upstream-spoolman.yml`): uses `actions/github-script` to avoid shell JSON-escaping issues. Creates the `upstream-watch` label automatically if it doesn't exist. Opens or updates one issue when tag or migration file list diverges from `upstream-pin.json`. Non-blocking — warnings via `core.notice`, never fails CI.

**`upstream-pin.json`** pinned at `v0.27.1` with the 15 migration `.py` filenames. Excludes `__init__.py` and `__pycache__`.

**`.gitignore`**: added `fixtures/` and `test-printers.local.toml` (not present on `main`; had been added only on `migration-test` branch).
