-- Migration 0006: single admin account, server-side sessions and API keys.
-- Tokens/keys are stored as SHA-256 hashes; the admin password as an Argon2id PHC string.
-- Applied from db.rs MIGRATIONS (CREATE TABLE IF NOT EXISTS, idempotent).

CREATE TABLE IF NOT EXISTS admin_user (
    id            INTEGER PRIMARY KEY NOT NULL CHECK (id = 1),
    username      TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS auth_session (
    id          INTEGER PRIMARY KEY NOT NULL,
    token_hash  TEXT NOT NULL UNIQUE,
    created_at  TEXT NOT NULL,
    expires_at  INTEGER NOT NULL,  -- unix seconds
    ip          TEXT,
    user_agent  TEXT
);

CREATE TABLE IF NOT EXISTS api_key (
    id           INTEGER PRIMARY KEY NOT NULL,
    name         TEXT NOT NULL,
    prefix       TEXT NOT NULL,
    key_hash     TEXT NOT NULL UNIQUE,
    created_at   TEXT NOT NULL,
    last_used_at TEXT,
    revoked_at   TEXT
);
