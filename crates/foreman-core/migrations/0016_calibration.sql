-- 0016: calibration history (task 5.4, W10). Applied from db.rs MIGRATIONS.
-- Skew belongs to a printer; shrinkage to a filament (its current value lives
-- in Spoolman's orcaslicer_config extra field, these rows are the history).

CREATE TABLE IF NOT EXISTS skew_calibration (
    id           INTEGER PRIMARY KEY NOT NULL,
    printer_id   INTEGER NOT NULL REFERENCES printer(id) ON DELETE CASCADE,
    planes_json  TEXT    NOT NULL,   -- [PlaneSkew]
    measured_at  TEXT    NOT NULL,
    measured_by  TEXT    NOT NULL,
    applied_at   TEXT,
    applied_by   TEXT,
    notes        TEXT
);
CREATE INDEX IF NOT EXISTS ix_skew_calibration_printer ON skew_calibration (printer_id);

CREATE TABLE IF NOT EXISTS shrinkage_calibration (
    id                     INTEGER PRIMARY KEY NOT NULL,
    filament_id            INTEGER NOT NULL,   -- Spoolman filament id (other DB)
    filament_name          TEXT    NOT NULL,
    printer_id             INTEGER REFERENCES printer(id) ON DELETE SET NULL,
    printer_name           TEXT,
    printer_skew_corrected INTEGER NOT NULL DEFAULT 0,
    input_json             TEXT    NOT NULL,
    shrinkage_xy           REAL,
    shrinkage_z            REAL,
    saved_at               TEXT    NOT NULL,
    saved_by               TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_shrinkage_calibration_filament ON shrinkage_calibration (filament_id);
