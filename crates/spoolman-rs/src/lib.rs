//! spoolman-rs — drop-in Rust implementation of the Spoolman HTTP + WebSocket API.
//!
//! Target compatibility: Spoolman v0.27.1 (`Donkie/Spoolman` tag v0.27.1).
//!
//! This crate is a scaffold stub. Phase 1 (sessions 4–10) fills in:
//! - Database layer (SQLite / Postgres / MySQL via sqlx), alembic-aware startup.
//! - Full REST API under `{base_path}/api/v1`.
//! - WebSocket subscription tree matching ws.py.
//! - Standalone binary + library mount inside foreman-server.

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {
        assert_eq!(2 + 2, 4);
    }
}
