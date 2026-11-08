# Claude Code brief — Phase 0 (scaffold) + Phase 1 (spoolman-rs)

Give this file to Claude Code in a **new, empty repo**, together with docs 01–03 from this project. Work session by session; tick boxes and commit after each one.

---

## Context (read first)
- Rewrite of `Hellsparks/Foreman-overview` (Node/Express + React). Old repo = **reference only**: read it for behaviour, never copy its structure.
- **Rule:** if it isn't frontend, it's Rust. Frontend = Vite + React + TypeScript, styled only with Fieldwork (`DESIGN.md`, `tokens.css` from `Hellsparks/Fieldwork-designtemplate`).
- Architecture (decided): one Rust core, three shells — `foreman-server` (headless, axum), `foreman-desktop` (Tauri 2, embeds the server or connects to a remote one), Tauri mobile later. All data over HTTP + one WebSocket event stream; Tauri IPC only for native features.
- No Docker self-management, no Python, no Node at runtime.
- v1 printer drivers: Klipper/Moonraker, OctoPrint, Duet, **Bambu (built last)** — Bambu supports Developer Mode (unsigned MQTT/FTPS) and signed mode for normal firmware (RSA-signed MQTT with a user-supplied cert/key loaded from config, never committed to the repo). Not in this brief.
- Owner's dev machine: CachyOS, KDE, Wayland, NVIDIA, **fish shell**. Second machine: Windows 11.

## Working rules
1. Every command in docs/README must work in **fish** and on Windows. Use `cargo` aliases (`.cargo/config.toml`) and `npm` scripts. No bash-only scripts, no `export VAR=…` instructions.
2. Before each commit: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, and frontend `npm run build` + tests if they were touched.
3. No `unwrap()`/`expect()` in server code paths. Errors via `thiserror`, logs via `tracing`.
4. Small commits, one checkbox (or a few) per commit. Update this file's checkboxes.
5. If something in this brief turns out wrong (API detail, upstream behaviour), **upstream source wins**; note the correction at the bottom of this file.

---

## Phase 0 — Scaffold

### 0.1 Repo layout
```
Cargo.toml                  (workspace)
crates/
  foreman-core/             (empty lib for now: config, error, event bus types)
  spoolman-rs/              (lib + bin `spoolman-rs`)
  foreman-server/           (bin: axum, serves API + frontend + mounts spoolman-rs)
  foreman-desktop/          (Tauri 2 app; src-tauri lives here)
frontend/                   (Vite + React + TS)
  src/design/               (copied Fieldwork tokens.css + DESIGN.md + AGENTS.md, unmodified)
docs/                       (docs 01–04 copied in)
.cargo/config.toml          (aliases)
.github/workflows/ci.yml
CLAUDE.md                   (rules above + pointers to docs/)
```
- [x] Workspace compiles; `cargo test` green with placeholder tests.
- [x] `CLAUDE.md` written: rules above, layout, "read docs/ before feature work", Fieldwork rules (tokens only, no other UI kit, no per-printer themes).

### 0.2 foreman-server skeleton
- [x] axum server: `GET /api/health` → `{status, version}`; static file serving of `frontend/dist` with SPA fallback.
- [x] `GET /ws/events` WebSocket that broadcasts from a `tokio::sync::broadcast` bus (send a heartbeat event every 10 s for now).
- [x] Config from file + env (port, data dir, DB URL). Defaults work with zero config. Data dir under the OS's standard app-data location.
- [x] Graceful shutdown on Ctrl+C / SIGTERM.

### 0.3 Frontend skeleton
- [x] Vite + React + TS app, imports `src/design/tokens.css` once; sets `data-fw-theme` / `data-fw-mode` on `<html>`; theme + mode switcher persisted locally.
- [x] Empty shell per doc 03 §4: left rail with 8 sections (Fleet, Jobs, Library, Projects, Filament, Service, Tools, Settings), top bar placeholders (search, Scan, Send, Attention), routes with placeholder pages, inspector slot on the right (closed).
- [x] API client module + WebSocket client hook that shows heartbeat connection state in the top bar.
- [x] Dev: Vite proxies `/api` and `/ws` to foreman-server.

### 0.4 Rust → TypeScript types
- [x] Use **ts-rs** on API DTOs (`#[derive(TS)]`), export to `frontend/src/api/types/` via a `cargo` alias (e.g. `cargo gen-types`). CI fails if generated types are out of date.
  - Why ts-rs: data travels over HTTP, not Tauri commands; ts-rs has stable releases. Specta/tauri-specta may be used later only for the few Tauri-native commands.

### 0.5 Tauri desktop shell
- [ ] Tauri 2 app loading the same frontend build.
- [ ] Mode A "this machine is the server": starts foreman-server in-process (tokio task) on a local port, then opens the window.
- [ ] Mode B "connect to server": URL field, stores choice, frontend talks to that URL.
- [ ] Builds on Linux (Wayland) and Windows. Note any NVIDIA/Wayland WebKitGTK workarounds needed in README.

### 0.6 CI
- [ ] GitHub Actions: fmt, clippy, test (Linux + Windows), frontend build, type-gen check.

**Phase 0 done =** `cargo run -p foreman-server` serves the empty shell with a live WS indicator; desktop app opens the same shell in both modes; CI green.

---

## Phase 1 — spoolman-rs (drop-in Spoolman replacement)

### Goal
A Rust implementation of Spoolman's HTTP + WebSocket API that existing clients cannot tell apart from the real one, running on an **existing Spoolman database unchanged**, on **SQLite, PostgreSQL and MySQL/MariaDB** (CockroachDB best effort via the Postgres driver).

Clients that must keep working: Moonraker `[spoolman]` (uses Spoolman's websocket + REST, incl. usage reports), Mainsail/Fluidd/KlipperScreen spool UIs (via Moonraker proxy), OctoPrint-Spoolman plugin, Home Assistant integrations, nfc2klipper-style tag readers (scan relay).

### Pinned upstream
- **Target: `Donkie/Spoolman` tag `v0.27.1`.** Clone it into `vendor/spoolman-upstream/` (gitignored, fetched by a script/alias) for reference and tests. Do not vendor its code into the build.
- Contract sources, in order of authority: `spoolman/api/v1/*.py` (routes, params, filters, sorting, pagination), `spoolman/database/*.py` (queries + weight logic), `spoolman/ws.py` (websocket subscription tree + event shape), `migrations/versions/*.py` (schema), `spoolman/env.py` (env vars), `tests_integration/`.
- Export upstream's OpenAPI JSON from a running upstream instance and commit it as `crates/spoolman-rs/tests/fixtures/openapi-v0.27.1.json`; add a test that diffs our route list against it.

### 1.1 Database layer
- [x] `sqlx` with SQLite, Postgres, MySQL, tls-rustls, any features; `Db` holds `AnyPool` + `Engine` enum; per-engine SQL in each `Migration`.
- [x] Schema identical to upstream at alembic head of v0.27.1 (tables: `vendor`, `filament`, `spool`, `setting`, `vendor_field`, `filament_field`, `spool_field`, `tag`, plus `alembic_version`).
- [x] Startup check: read `alembic_version`.
  - equals our known head → run.
  - older known revision → apply the equivalent migrations (ported per engine) and write the same `alembic_version` values Alembic would, so the DB can still be opened by Python Spoolman.
  - unknown/newer → refuse to start with a clear message.
- [ ] Fresh DB created by spoolman-rs must be openable by upstream Python Spoolman v0.27.1 (test this — deferred to 1.5 integration gate, requires a running Python Spoolman).
- [x] Respect upstream env vars (`SPOOLMAN_DB_TYPE`, `SPOOLMAN_DB_HOST`, `_PORT`, `_NAME`, `_USERNAME`, `_PASSWORD`/`_PASSWORD_FILE`, `SPOOLMAN_DIR_DATA`, `SPOOLMAN_BASE_PATH`, `SPOOLMAN_HOST`, `SPOOLMAN_PORT`, `SPOOLMAN_AUTOMATIC_BACKUP`, `SPOOLMAN_DIR_LOGS`, `SPOOLMAN_DIR_BACKUPS` — full list confirmed from `env.py`). Default SQLite path matches upstream's default location (`~/.local/share/spoolman/spoolman.db` on Linux).

### 1.2 REST API (`{base_path}/api/v1`)
- [ ] vendor, filament, spool CRUD with upstream filtering (comma lists, empty-value semantics, extra-field filters), sorting, pagination (incl. total-count header — check upstream for exact name).
- [ ] spool `use` (by weight and by length → weight via density/diameter) and `measure`; first/last used timestamps; archived.
- [ ] extra fields (`/field/{entity}/{key}`): definitions in `setting`, values JSON-encoded in `*_field` tables, validation per field type.
- [ ] settings, info/health, backup, find endpoints (material, location, lot-number, article-number), location rename, export, search, externaldb (SpoolmanDB cache/proxy), tags (NFC/OpenPrintTag) and scan relay.
- [ ] Error responses match upstream status codes and body shape.

### 1.3 WebSockets
- [ ] Same paths and subscription tree as `ws.py` (all-events root, per-collection, per-item), same event JSON (type, resource, date, payload), same triggers (added/updated/deleted, including spool updates caused by filament changes).
- [ ] Scan relay websockets with upstream debounce (3 s) and reader registry semantics.

### 1.4 Packaging
- [ ] Standalone binary `spoolman-rs` (for people who only want the Spoolman replacement).
- [ ] Library mount inside foreman-server on **its own listener port** (configurable) so Moonraker can point at `http://host:<port>` exactly like upstream. Foreman's own UI talks to it in-process.
- [ ] No web UI. `GET /` returns a small page pointing to Foreman's Filament section (or a JSON info doc when Accept is JSON).

### 1.5 Compatibility gate (definition of done)
- [ ] Upstream `tests_integration/` suite runs against spoolman-rs (it targets `SPOOLMAN_HOST`/`SPOOLMAN_PORT`) via a cargo alias that starts DB containers (Docker is fine **for tests only**). Green on **SQLite, Postgres, MariaDB**; Cockroach run reported, not blocking.
- [ ] Route-list diff vs committed OpenAPI fixture = empty.
- [ ] Migration test: copy of a real older Spoolman SQLite DB (owner will provide one) → starts, migrates, data intact, opens in upstream Python afterwards.
- [ ] Moonraker smoke test: real or containerised Moonraker with `[spoolman] server: http://…` shows `spoolman_connected: true`, active spool set/cleared, usage reports decrease `used_weight`.
- [ ] Load test: 50 concurrent clients with websockets; memory and latency recorded vs upstream (report only).

### 1.6 Foreman-specific extra fields
- [ ] On first Foreman start (not in standalone mode), ensure Foreman's extra fields exist exactly as Marathon created them: `url`, `hue_td`, `swatch`, `orcaslicer_config`, `material_modifier` — read `backend/src/routes/setup.js` in the old repo for entity + type of each.

---

## Session plan
| Session | Boxes |
|---|---|
| 1 | 0.1, 0.2 |
| 2 | 0.3, 0.4 |
| 3 | 0.5, 0.6 |
| 4 | 1.1 (SQLite) + startup revision check |
| 5 | 1.1 (Postgres, MySQL) + migration ports |
| 6 | 1.2 vendor/filament/spool + filters/sort/paging |
| 7 | 1.2 use/measure, extra fields, settings, find/export |
| 8 | 1.2 search, externaldb, tags + 1.3 websockets |
| 9 | 1.4 packaging + 1.5 test harness |
| 10 | 1.5 fix-until-green + Moonraker smoke + 1.6 |

## Corrections log
(Claude Code: record any place where this brief disagreed with upstream source, and what you did.)

### Session 1 — 2026-11-08

**Broken rustup shim on dev machine (not a brief issue — environment note):**
`/usr/bin/cargo` and `/usr/bin/rustc` are symlinks to `/usr/bin/rustup`, which is a Pacman-managed shim that fails with `error: unknown proxy name: 'open_codesign'`. The actual working toolchain is at `~/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/`. All `cargo` commands in this session were run with the full path: `~/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo`.

**Fix for future sessions:** add `~/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin` to `$PATH` before the system path in `~/.config/fish/config.fish`. Suggested line:
```fish
fish_add_path --prepend ~/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin
```
After that, `cargo` and `rustc` resolve to the working toolchain.

**ts-rs chrono feature not in scaffold brief:** `ts-rs` v10 requires the `chrono-impl` feature to derive `TS` for `DateTime<Utc>`. Added `chrono-impl` to `ts-rs` workspace dependency. Not a contradiction with upstream — just a dependency detail not mentioned in the doc.

### Session 4 — 2026-11-08

**`RUSTUP_TOOLCHAIN=stable` required for `cargo test`:** On this machine rustup attempts to auto-install old toolchain versions (1.57.0) during the build, which causes `--check-cfg` to be rejected. All `cargo` commands must be prefixed with `RUSTUP_TOOLCHAIN=stable` (or the full toolchain path) to prevent this.

**Migration SQL split strategy:** sqlx's `execute()` on SQLite only processes a single statement per call. Multi-statement migration SQL blocks must be split on `;` and executed one statement at a time. Implemented in `migrations.rs::apply_from`.

**`92793c8a937c` color_hex alpha migration is a no-op for SQLite:** Upstream uses an Alembic `batch_alter_table` to widen `color_hex` from 6 to 8 chars. SQLite does not enforce VARCHAR lengths, so the migration records the revision but runs no SQL. The column accepts 8-char values regardless.

**"Fresh DB openable by Python Spoolman" test deferred:** This requires a running Python Spoolman instance. Deferred to section 1.5 (compatibility gate), which already calls for running `tests_integration/` against a live instance.

**`AnyPool::connect` does not set SQLite `create_if_missing`:** Unlike `SqlitePool::connect_with(opts.create_if_missing(true))`, `AnyPool::connect(url)` opens SQLite in read-write-only mode (no create). `From<SqliteConnectOptions>` is not implemented for `AnyConnectOptions` in sqlx 0.8. Workaround: `std::fs::File::create` the SQLite file explicitly before calling `AnyPool::connect`, then run WAL/FK PRAGMA commands as the first query.

**SQLite error code 1 is generic:** `is_table_not_found` must not short-circuit on a non-nil error code — SQLite code 1 (`SQLITE_ERROR`) is used for many errors including "no such table". Logic changed to check Postgres/MySQL specific codes first, then always fall through to a message-prefix check.
