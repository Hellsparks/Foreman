-- Migration 0012: templates and projects (task 5.1, W9).
-- Applied from db.rs MIGRATIONS (this file documents it).
--
-- template: one row per template; slots and plates live in body_json
--   {slots:[{key,label,hex,material,allow_swap}], plates:[{name,file_id,copies,tools:[{tool,slot}]}]}
--   (edited as one document; plate files are copies under Library "Templates/<name>/").
-- project: a run of a template. status: active | done | archived.
-- project_slot: colour slot → the Spoolman spool feeding it.
-- project_plate: copies = prints wanted in total (template copies × sets, editable,
--   never below printed + printing); printed = finished prints (job engine hook).
--   tools_json = [{tool, slot}] (slot = project_slot.key).
-- project_event: activity log.
-- queue_item.project_plate_id / job.project_plate_id: which plate a print is for.

CREATE TABLE IF NOT EXISTS template (
    id            INTEGER PRIMARY KEY NOT NULL,
    name          TEXT NOT NULL,
    category      TEXT,
    notes         TEXT,
    thumb_file_id INTEGER,
    body_json     TEXT NOT NULL DEFAULT '{}',
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS project (
    id            INTEGER PRIMARY KEY NOT NULL,
    name          TEXT NOT NULL,
    template_id   INTEGER REFERENCES template(id) ON DELETE SET NULL,
    template_name TEXT,
    sets          INTEGER NOT NULL DEFAULT 1,
    due_date      TEXT,
    status        TEXT NOT NULL DEFAULT 'active',
    created_at    TEXT NOT NULL,
    closed_at     TEXT
);
CREATE TABLE IF NOT EXISTS project_slot (
    id         INTEGER PRIMARY KEY NOT NULL,
    project_id INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    key        TEXT NOT NULL,
    label      TEXT,
    hex        TEXT,
    spool_id   INTEGER,
    position   INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS project_plate (
    id         INTEGER PRIMARY KEY NOT NULL,
    project_id INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    file_id    INTEGER NOT NULL,
    copies     INTEGER NOT NULL DEFAULT 1,
    printed    INTEGER NOT NULL DEFAULT 0,
    tools_json TEXT NOT NULL DEFAULT '[]',
    position   INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS project_event (
    id         INTEGER PRIMARY KEY NOT NULL,
    project_id INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    at         TEXT NOT NULL,
    text       TEXT NOT NULL
);
ALTER TABLE queue_item ADD COLUMN project_plate_id INTEGER;
ALTER TABLE job ADD COLUMN project_plate_id INTEGER;
