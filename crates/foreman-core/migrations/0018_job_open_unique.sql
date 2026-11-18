-- Migration 0018: one open job per printer.
-- A printer that reports "printing" within milliseconds of Foreman's start command
-- made the start path and the status path each open a job. The check-then-insert in
-- Rust was not exclusive, so the database now refuses a second running job for the
-- same printer; the loser attaches to the winner's row (repo::job::open).
-- Existing duplicates: keep the oldest running job per printer, close the others as
-- errors with deduction skipped (so filament is never deducted twice).
-- Jobs whose printer was deleted (printer_id NULL) are not constrained.
-- Applied from db.rs MIGRATIONS (this file documents it).

UPDATE job SET end_state = 'error', ended_at = COALESCE(ended_at, started_at),
    error = 'duplicate job opened by a start race (merged by migration 0018)',
    deduct_state = 'skipped'
WHERE end_state = 'running' AND printer_id IS NOT NULL
  AND id NOT IN (SELECT MIN(id) FROM job WHERE end_state = 'running' GROUP BY printer_id);

CREATE UNIQUE INDEX IF NOT EXISTS ux_job_open_printer
    ON job (printer_id) WHERE end_state = 'running';
