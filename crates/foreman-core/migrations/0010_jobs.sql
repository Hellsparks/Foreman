-- Migration 0010: job engine (task 3.6).
-- job: one row per print, opened when the print starts (by Foreman or seen on the
--   printer) and closed when it ends. `end` is what the firmware reported
--   (running | complete | cancelled | error); `outcome` is the user's verdict
--   (NULL = not rated yet | good | failed | cancelled).
-- deduct_state: none → pending → done | skipped. Moving none → pending is a
--   conditional update, so filament is deducted at most once per job, even across
--   restarts (a crash between pending and done never deducts again).
-- tools_json: per tool {index, material, colour, grams_planned, grams_deducted, spool_id}.
-- printer.usage_reporter: auto | moonraker | foreman.
-- attention_item.job_id: the job a print_finished item is about.
-- Applied from db.rs MIGRATIONS (this file documents it).

CREATE TABLE IF NOT EXISTS job (
    id             INTEGER PRIMARY KEY NOT NULL,
    printer_id     INTEGER REFERENCES printer(id) ON DELETE SET NULL,
    printer_name   TEXT    NOT NULL,
    file_id        INTEGER,
    file_name      TEXT    NOT NULL,
    queue_item_id  INTEGER,
    started_at     TEXT    NOT NULL,
    ended_at       TEXT,
    duration_secs  INTEGER,
    end_state      TEXT    NOT NULL DEFAULT 'running',
    outcome        TEXT,
    fail_reason    TEXT,
    error          TEXT,
    usage_reporter TEXT    NOT NULL DEFAULT 'foreman',
    grams_planned  REAL,
    grams_used     REAL,
    grams_deducted REAL    NOT NULL DEFAULT 0,
    material       TEXT,
    tools_json     TEXT    NOT NULL DEFAULT '[]',
    deduct_state   TEXT    NOT NULL DEFAULT 'none',
    thumb_url      TEXT
);
CREATE INDEX IF NOT EXISTS ix_job_printer ON job (printer_id);
CREATE INDEX IF NOT EXISTS ix_job_started ON job (started_at);
ALTER TABLE printer ADD COLUMN usage_reporter TEXT NOT NULL DEFAULT 'auto';
ALTER TABLE attention_item ADD COLUMN job_id INTEGER;
