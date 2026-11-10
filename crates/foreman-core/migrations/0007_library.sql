-- 0007: Library (task 3.1). Applied from db.rs MIGRATIONS; kept here for reference.
-- Files live under <data_dir>/library; this is the index. Paths are relative and
-- `/`-separated. Trashed entries keep their original path (for restore) and share a
-- trash_id with everything deleted in the same action; trash_root marks the top entry.

CREATE TABLE IF NOT EXISTS library_entry (
    id          INTEGER PRIMARY KEY NOT NULL,
    path        TEXT NOT NULL,
    parent      TEXT NOT NULL,               -- '' for the library root
    kind        TEXT NOT NULL,               -- 'file' | 'folder'
    size        INTEGER NOT NULL DEFAULT 0,
    hash        TEXT,                        -- sha256 hex
    mtime       INTEGER,                     -- unix seconds, for index rebuild
    added_at    TEXT NOT NULL,
    starred     INTEGER NOT NULL DEFAULT 0,
    deleted_at  TEXT,
    trash_id    TEXT,
    trash_root  INTEGER NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX IF NOT EXISTS library_entry_live_path
    ON library_entry(path) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS library_entry_parent ON library_entry(parent);

CREATE TABLE IF NOT EXISTS file_meta (
    entry_id             INTEGER PRIMARY KEY NOT NULL REFERENCES library_entry(id) ON DELETE CASCADE,
    format               TEXT NOT NULL,      -- gcode | bgcode | 3mf | stl | other
    parse_status         TEXT NOT NULL,      -- ok | unsupported | error
    parse_error          TEXT,
    slicer               TEXT,
    slicer_version       TEXT,
    printer_model        TEXT,
    printer_settings_id  TEXT,               -- slicer printer profile name
    printer_model_id     TEXT,               -- Bambu model code from slice_info.config (e.g. N2S)
    nozzle_diameter      REAL,
    print_time_secs      INTEGER,
    layer_count          INTEGER,
    bed_width            REAL,
    bed_depth            REAL,
    filament_settings_id TEXT,
    suggested_spool_id   INTEGER,
    tools_json           TEXT NOT NULL DEFAULT '[]',
    plates_json          TEXT NOT NULL DEFAULT '[]',
    thumb_file           TEXT,               -- file name under <data_dir>/library-cache/thumbs
    parsed_at            TEXT NOT NULL
);
