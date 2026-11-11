-- 0015: open Attention items are unique per (printer, kind) — session 32.
-- `kind` carries the subject where a printer can have several
-- (e.g. maintenance:<task_id>). Every restart used to add another
-- "<printer>: offline" item because the in-memory dedupe map was empty.
-- Applied from db.rs MIGRATIONS (this file documents it).

UPDATE attention_item SET cleared_at = COALESCE(cleared_at, created_at)
WHERE cleared_at IS NULL AND id NOT IN (
  SELECT MIN(id) FROM attention_item WHERE cleared_at IS NULL
  GROUP BY COALESCE(printer_id, -1), kind);

CREATE UNIQUE INDEX IF NOT EXISTS ux_attention_item_open
  ON attention_item (COALESCE(printer_id, -1), kind) WHERE cleared_at IS NULL;
