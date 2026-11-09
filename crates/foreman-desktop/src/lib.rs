//! foreman-desktop — Tauri 2 desktop shell.
//!
//! Mode A embeds foreman-server in-process; Mode B connects to a remote server
//! (mDNS discovery, address probing, saved servers with bearer tokens).

pub mod discover;
pub mod probe;
pub mod store;
