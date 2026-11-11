-- Migration 0009: printer queues + fleet queue (task 3.5).
-- printer_id NULL = fleet queue; once a fleet item is dispatched, printer_id is the
-- printer it went to and from_fleet = 1.
-- status: waiting | sending | printing | done | failed | cancelled.
-- Applied from db.rs MIGRATIONS (this file documents it).

CREATE TABLE IF NOT EXISTS queue_item (
    id          INTEGER PRIMARY KEY NOT NULL,
    file_id     INTEGER NOT NULL,
    file_name   TEXT    NOT NULL,
    source      TEXT    NOT NULL DEFAULT 'manual',
    added_by    TEXT    NOT NULL,
    added_at    TEXT    NOT NULL,
    status      TEXT    NOT NULL DEFAULT 'waiting',
    printer_id  INTEGER REFERENCES printer(id) ON DELETE SET NULL,
    from_fleet  INTEGER NOT NULL DEFAULT 0,
    position    INTEGER NOT NULL DEFAULT 0,
    error       TEXT,
    updated_at  TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_queue_item_status ON queue_item (status);
