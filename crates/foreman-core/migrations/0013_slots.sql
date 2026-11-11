-- Migration 0013: filament slots per file (task 5.2, W9).
-- Applied from db.rs MIGRATIONS (this file documents it).
-- slot_kind: tool | ams | change; slots_json: [FilamentSlot]. NULL = parsed before
-- 0013; the server re-parses those files once at startup.
ALTER TABLE file_meta ADD COLUMN slot_kind TEXT;
ALTER TABLE file_meta ADD COLUMN slots_json TEXT;
