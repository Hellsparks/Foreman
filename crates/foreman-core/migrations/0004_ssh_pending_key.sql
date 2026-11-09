-- Migration 0004: SSH host key presented after a change, held until the user trusts it
-- (TOFU refuse-on-change). Applied from db.rs MIGRATIONS.

ALTER TABLE printer_ssh ADD COLUMN pending_host_key_fingerprint TEXT;
