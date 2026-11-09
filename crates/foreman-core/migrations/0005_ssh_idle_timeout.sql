-- Migration 0005: idle auto-close for pooled SSH/SFTP sessions and terminals
-- (seconds, 0 = never). Applied from db.rs MIGRATIONS.

ALTER TABLE printer_ssh ADD COLUMN idle_timeout_secs INTEGER NOT NULL DEFAULT 900;
