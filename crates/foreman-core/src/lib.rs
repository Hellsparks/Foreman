//! foreman-core — shared domain types, error definitions, and event bus types.
//!
//! This crate is intentionally thin at the scaffold stage. It will grow to hold:
//! - Domain types (printer state, job, spool, filament, etc.)
//! - `AppError` via `thiserror`
//! - Event bus payload types (broadcast over `/ws/events`)
//! - Configuration types (shared between server and desktop)

pub mod error;
pub mod events;

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {
        assert_eq!(2 + 2, 4);
    }
}
