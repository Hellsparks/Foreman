-- Migration 0003: add failover timing settings to the printer table.
-- fail_after_checks: how many consecutive health-check failures on the active
--   path before Foreman switches to the next path (default 3, ≈6 s at 2s interval).
-- return_after_secs: how long a higher-priority path must be healthy before
--   Foreman switches back (default 60 s).

ALTER TABLE printer ADD COLUMN fail_after_checks INTEGER NOT NULL DEFAULT 3;
ALTER TABLE printer ADD COLUMN return_after_secs  INTEGER NOT NULL DEFAULT 60;
