-- Migration 0019: Bambu Cloud accounts (doc 07 §B, session "Bambu accounts + sign-in").
-- Applied from db.rs MIGRATIONS (CREATE TABLE IF NOT EXISTS / ALTER ADD COLUMN, idempotent).
--
-- Many accounts by design: nothing assumes one. Each row has its own token, expiry and
-- error; one failing never touches another.
-- token: the Bambu access token sealed with secret.key (`enc:v1:…`, see secrets.rs). The
--   password is never stored anywhere.
-- token_expires_at: RFC 3339, NULL when Bambu did not say (pasted opaque token).
-- last_ok_at / last_error: last successful Bambu call / last failure text, per account.
-- printer.bambu_account_id: which account a printer belongs to (linking UI comes later).

CREATE TABLE IF NOT EXISTS bambu_account (
    id               INTEGER PRIMARY KEY NOT NULL,
    label            TEXT NOT NULL,
    region           TEXT NOT NULL CHECK (region IN ('global', 'china')),
    email            TEXT NOT NULL,
    token            TEXT,
    token_expires_at TEXT,
    last_ok_at       TEXT,
    last_error       TEXT,
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL
);

ALTER TABLE printer ADD COLUMN bambu_account_id INTEGER
    REFERENCES bambu_account(id) ON DELETE SET NULL;
