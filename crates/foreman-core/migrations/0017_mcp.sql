-- Migration 0017: MCP access per API key (task 6.5, W14).
-- Applied from db.rs MIGRATIONS (CREATE TABLE IF NOT EXISTS, idempotent).
--
-- No row for a key = the defaults: every read-only tool, no destructive tools, 60 calls/min.
-- tools_json: JSON array of tool names the key may call; NULL = every read-only tool.
-- allow_destructive: destructive tools (move a printer, start/cancel prints, delete data,
--   change filament counts) are refused unless this is 1 AND the tool is in tools_json.

CREATE TABLE IF NOT EXISTS mcp_key_policy (
    api_key_id        INTEGER PRIMARY KEY NOT NULL REFERENCES api_key(id) ON DELETE CASCADE,
    tools_json        TEXT,
    allow_destructive INTEGER NOT NULL DEFAULT 0,
    rate_per_minute   INTEGER NOT NULL DEFAULT 60,
    updated_at        TEXT NOT NULL
);
