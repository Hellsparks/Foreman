-- Migration 0014: maintenance (task 5.3, W12).
-- Applied from db.rs MIGRATIONS (this file documents it).
-- maintenance_task: one recurring service per printer.
--   basis: hours | grams | jobs — what the interval counts. Counters are not
--   stored; they are summed from `job` rows ended after `since` (NULL = the
--   printer's whole job history), never from cancelled jobs.
--   every_days: optional calendar interval; the task is due on whichever comes first.
--   snooze: extra allowance as a fraction of the interval, cleared when done.
-- maintenance_log: one row per "mark done", with the counter value at the time.
--   prev_since / prev_snooze restore the task when the latest done is undone.
CREATE TABLE IF NOT EXISTS maintenance_task (
    id         INTEGER PRIMARY KEY NOT NULL,
    printer_id INTEGER NOT NULL REFERENCES printer(id) ON DELETE CASCADE,
    name       TEXT    NOT NULL,
    notes      TEXT,
    basis      TEXT    NOT NULL,
    interval   REAL    NOT NULL,
    every_days INTEGER,
    snooze     REAL    NOT NULL DEFAULT 0,
    since      TEXT,
    created_at TEXT    NOT NULL,
    updated_at TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_maintenance_task_printer ON maintenance_task (printer_id);
CREATE TABLE IF NOT EXISTS maintenance_log (
    id          INTEGER PRIMARY KEY NOT NULL,
    task_id     INTEGER REFERENCES maintenance_task(id) ON DELETE SET NULL,
    printer_id  INTEGER NOT NULL REFERENCES printer(id) ON DELETE CASCADE,
    task_name   TEXT    NOT NULL,
    basis       TEXT    NOT NULL,
    interval    REAL    NOT NULL,
    counter     REAL    NOT NULL,
    days        REAL    NOT NULL,
    done_at     TEXT    NOT NULL,
    done_by     TEXT    NOT NULL,
    notes       TEXT,
    prev_since  TEXT,
    prev_snooze REAL    NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS ix_maintenance_log_task ON maintenance_log (task_id);
CREATE INDEX IF NOT EXISTS ix_maintenance_log_printer ON maintenance_log (printer_id);
