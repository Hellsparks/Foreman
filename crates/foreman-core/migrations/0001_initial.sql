-- Foreman-native database schema (SQLite only).
-- Migration 0001: initial tables for Phase 2.

PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;

-- ---------------------------------------------------------------------------
-- printer
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS printer (
    id               INTEGER PRIMARY KEY NOT NULL,
    name             TEXT    NOT NULL,
    firmware         TEXT    NOT NULL,  -- moonraker | octoprint | bambu | duet
    primary_url      TEXT,
    api_key          TEXT,
    serial_number    TEXT,              -- Bambu serial
    access_code      TEXT,              -- Bambu LAN access code
    tags             TEXT    NOT NULL DEFAULT '[]',  -- JSON array of strings
    hardened_nozzle  INTEGER NOT NULL DEFAULT 0,     -- bool (0/1)
    sort_order       INTEGER NOT NULL DEFAULT 0,
    created_at       TEXT    NOT NULL,
    updated_at       TEXT    NOT NULL
);

-- ---------------------------------------------------------------------------
-- printer_path  (tiered connection paths, ordered by priority asc)
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS printer_path (
    id          INTEGER PRIMARY KEY NOT NULL,
    printer_id  INTEGER NOT NULL REFERENCES printer(id) ON DELETE CASCADE,
    name        TEXT    NOT NULL,
    url         TEXT    NOT NULL,
    priority    INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT    NOT NULL,
    updated_at  TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS ix_printer_path_printer_id ON printer_path (printer_id);

-- ---------------------------------------------------------------------------
-- printer_ssh
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS printer_ssh (
    id                   INTEGER PRIMARY KEY NOT NULL,
    printer_id           INTEGER NOT NULL UNIQUE REFERENCES printer(id) ON DELETE CASCADE,
    ssh_user             TEXT    NOT NULL,
    ssh_port             INTEGER NOT NULL DEFAULT 22,
    auth_type            TEXT    NOT NULL DEFAULT 'key',  -- key | password
    password             TEXT,                             -- encrypted, only for auth_type=password
    host_key_fingerprint TEXT,
    created_at           TEXT    NOT NULL,
    updated_at           TEXT    NOT NULL
);

-- ---------------------------------------------------------------------------
-- webcam
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS webcam (
    id          INTEGER PRIMARY KEY NOT NULL,
    printer_id  INTEGER NOT NULL REFERENCES printer(id) ON DELETE CASCADE,
    name        TEXT    NOT NULL,
    url         TEXT    NOT NULL,
    stream_type TEXT    NOT NULL DEFAULT 'mjpeg',  -- mjpeg | webrtc | rtsps
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS ix_webcam_printer_id ON webcam (printer_id);

-- ---------------------------------------------------------------------------
-- macro_favourite  (per-printer Klipper macro favourites)
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS macro_favourite (
    id          INTEGER PRIMARY KEY NOT NULL,
    printer_id  INTEGER NOT NULL REFERENCES printer(id) ON DELETE CASCADE,
    macro_name  TEXT    NOT NULL,
    UNIQUE (printer_id, macro_name)
);

-- ---------------------------------------------------------------------------
-- temp_preset  (temperature presets per printer)
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS temp_preset (
    id            INTEGER PRIMARY KEY NOT NULL,
    printer_id    INTEGER NOT NULL REFERENCES printer(id) ON DELETE CASCADE,
    name          TEXT    NOT NULL,
    extruder_temp INTEGER,
    bed_temp      INTEGER,
    chamber_temp  INTEGER,
    sort_order    INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS ix_temp_preset_printer_id ON temp_preset (printer_id);

-- ---------------------------------------------------------------------------
-- shell_macro  (named multi-line SSH commands)
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS shell_macro (
    id             INTEGER PRIMARY KEY NOT NULL,
    name           TEXT    NOT NULL,
    commands       TEXT    NOT NULL,  -- multi-line body
    scope          TEXT    NOT NULL DEFAULT 'printer',  -- printer | all_ssh
    printer_id     INTEGER REFERENCES printer(id) ON DELETE CASCADE,  -- NULL for scope=all_ssh
    ask_before_run INTEGER NOT NULL DEFAULT 0,
    created_at     TEXT    NOT NULL,
    updated_at     TEXT    NOT NULL
);

-- ---------------------------------------------------------------------------
-- audit_log  (terminal commands and macro runs)
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS audit_log (
    id          INTEGER PRIMARY KEY NOT NULL,
    printer_id  INTEGER REFERENCES printer(id) ON DELETE SET NULL,
    user        TEXT,
    command     TEXT NOT NULL,
    occurred_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS ix_audit_log_printer_id  ON audit_log (printer_id);
CREATE INDEX IF NOT EXISTS ix_audit_log_occurred_at ON audit_log (occurred_at);

-- ---------------------------------------------------------------------------
-- attention_item
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS attention_item (
    id          INTEGER PRIMARY KEY NOT NULL,
    printer_id  INTEGER REFERENCES printer(id) ON DELETE CASCADE,
    kind        TEXT    NOT NULL,   -- print_finished | error | paused | runout | spool_low |
                                    -- maintenance | offline | path_switched | upload_failed
    title       TEXT    NOT NULL,
    body        TEXT,
    actions     TEXT    NOT NULL DEFAULT '[]',  -- JSON array
    cleared_at  TEXT,
    created_at  TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS ix_attention_item_printer_id ON attention_item (printer_id);
CREATE INDEX IF NOT EXISTS ix_attention_item_cleared_at ON attention_item (cleared_at);

-- ---------------------------------------------------------------------------
-- setting  (Foreman-native key-value store)
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS setting (
    key        TEXT PRIMARY KEY NOT NULL,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
