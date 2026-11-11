-- Migration 0008: slicer profiles per printer + plate-clear state (tasks 3.3 + 3.4).
-- slicer_profiles: JSON array of slicer printer-profile names that map to this printer
--   (e.g. ["Voron V0 0.4mm", "Voron V0"]). The compatibility engine matches the file's
--   printer_settings_id against these.
-- plate_clear: whether the print bed is ready for the next job (starts true so the
--   first job can go; set false after a print finishes, set true by the user or
--   attention action "Plate cleared").
-- auto_start_from_queue: per-printer setting; when off, the scheduler skips this printer.

ALTER TABLE printer ADD COLUMN slicer_profiles TEXT NOT NULL DEFAULT '[]';
ALTER TABLE printer ADD COLUMN plate_clear INTEGER NOT NULL DEFAULT 1;
ALTER TABLE printer ADD COLUMN auto_start_from_queue INTEGER NOT NULL DEFAULT 1;
