-- Migration 0002: add TLS certificate fingerprint pinning for Bambu printers.
-- On first TLS connect, the SHA-256 fingerprint of the printer's cert is stored.
-- Later connects must match; on mismatch the driver refuses and publishes a
-- printer.connection event with reason "certificate changed".

ALTER TABLE printer ADD COLUMN tls_fingerprint TEXT;
