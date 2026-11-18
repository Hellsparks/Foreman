-- Migration 0020: Bambu Cloud transport (doc 07 §B, session "Cloud transport").
-- Set when Bambu refuses the stored token (cloud MQTT or REST). The account's cloud
-- connection stops retrying and one Attention item asks for a re-authentication.
-- Storing a new token clears it.
ALTER TABLE bambu_account ADD COLUMN token_rejected_at TEXT;
