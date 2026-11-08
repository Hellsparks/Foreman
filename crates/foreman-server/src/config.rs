use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Server configuration.
///
/// Values are loaded in priority order:
/// 1. Environment variables (prefixed `FOREMAN_`)
/// 2. Config file at `FOREMAN_CONFIG_FILE` (default: `<data_dir>/config.toml`)
/// 3. Built-in defaults below
///
/// All env-var overrides use `FOREMAN__` double-underscore nesting,
/// e.g. `FOREMAN__PORT=9091`, `FOREMAN__DB__URL=sqlite:///tmp/foreman.db`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// TCP port for the HTTP + WebSocket server. Default: `9090`.
    #[serde(default = "default_port")]
    pub port: u16,

    /// Host address to bind. Default: `127.0.0.1`.
    #[serde(default = "default_host")]
    pub host: String,

    /// Directory for data files (SQLite DB, uploads, backups, etc.).
    /// Default: OS app-data location (see [`default_data_dir`]).
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,

    /// Database connection URL.
    /// Default: `sqlite://<data_dir>/foreman.db`.
    /// Overridden at runtime if still `None` after loading.
    pub db_url: Option<String>,

    /// Whether to run the Spoolman-compatible API alongside Foreman.
    /// Default: `true`. Disable with `FOREMAN__SPOOLMAN_ENABLED=false`.
    #[serde(default = "default_spoolman_enabled")]
    pub spoolman_enabled: bool,
}

fn default_port() -> u16 {
    9090
}

fn default_spoolman_enabled() -> bool {
    true
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

/// Returns the OS-standard application data directory for Foreman.
///
/// | Platform | Path |
/// |----------|------|
/// | Linux    | `$XDG_DATA_HOME/foreman` or `~/.local/share/foreman` |
/// | macOS    | `~/Library/Application Support/foreman` |
/// | Windows  | `%APPDATA%\foreman` |
pub fn default_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("foreman")
}

impl ServerConfig {
    /// Load config from environment + optional config file.
    ///
    /// Never panics; falls back to defaults on any error.
    pub fn load() -> Self {
        let mut builder = config::Config::builder();

        // 1. Defaults
        builder = builder
            .set_default("port", 9090u16)
            .expect("default port is valid")
            .set_default("host", "127.0.0.1")
            .expect("default host is valid");

        // 2. Config file (optional — missing file is not an error)
        let config_path = std::env::var("FOREMAN_CONFIG_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_data_dir().join("config.toml"));

        builder = builder.add_source(config::File::from(config_path).required(false));

        // 3. Environment variables: FOREMAN__PORT, FOREMAN__DB__URL, etc.
        builder = builder.add_source(config::Environment::with_prefix("FOREMAN").separator("__"));

        let raw = builder.build().unwrap_or_else(|err| {
            tracing::warn!("config load error (using defaults): {err}");
            config::Config::default()
        });

        let mut cfg: ServerConfig = raw.try_deserialize().unwrap_or_else(|err| {
            tracing::warn!("config deserialise error (using defaults): {err}");
            ServerConfig::default()
        });

        // Derive DB URL from data_dir if not explicitly set
        if cfg.db_url.is_none() {
            cfg.db_url = Some(format!(
                "sqlite://{}",
                cfg.data_dir.join("foreman.db").display()
            ));
        }

        cfg
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        let data_dir = default_data_dir();
        let db_url = format!("sqlite://{}", data_dir.join("foreman.db").display());
        Self {
            port: default_port(),
            host: default_host(),
            db_url: Some(db_url),
            data_dir,
            spoolman_enabled: default_spoolman_enabled(),
        }
    }
}
