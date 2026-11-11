-- Migration 0011: tracked filaments (task 4.3).
-- One row per Spoolman filament Foreman tracks stock for. `filament_id` is the
-- Spoolman filament id (no FK: Spoolman is a separate database). Rows for
-- filaments that no longer exist are ignored and removed on read.
-- target    = how many spools to have on hand,
-- buy_below = put it on the shopping list when usable spools drop below this.
-- Applied from db.rs MIGRATIONS (this file documents it).

CREATE TABLE IF NOT EXISTS tracked_filament (
    filament_id INTEGER PRIMARY KEY NOT NULL,
    target      INTEGER NOT NULL,
    buy_below   INTEGER NOT NULL,
    updated_at  TEXT    NOT NULL
);
