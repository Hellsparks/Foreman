use std::{env, fs, path::PathBuf};

use crate::error::{Result, SpoolmanError};

/// Database back-end.
///
/// Session 4 only compiles SQLite; Postgres / MySQL are added in session 5.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbType {
    Sqlite,
    // Postgres,  // session 5
    // Mysql,     // session 5
}

/// All configuration that spoolman-rs reads from SPOOLMAN_* environment variables.
///
/// Mirrors `spoolman/env.py` from the upstream v0.27.1 release; every variable
/// that upstream reads is represented here so that existing deployments just work.
#[derive(Debug, Clone)]
pub struct SpoolmanConfig {
    /// Which database back-end to use (`SPOOLMAN_DB_TYPE`; default sqlite).
    pub db_type: DbType,

    // --- remote-DB fields (Postgres / MySQL, ignored for SQLite) ---
    /// `SPOOLMAN_DB_HOST`
    pub db_host: Option<String>,
    /// `SPOOLMAN_DB_PORT`
    pub db_port: Option<u16>,
    /// `SPOOLMAN_DB_NAME`
    pub db_name: Option<String>,
    /// `SPOOLMAN_DB_USERNAME`
    pub db_username: Option<String>,
    /// `SPOOLMAN_DB_PASSWORD` / `SPOOLMAN_DB_PASSWORD_FILE`
    pub db_password: Option<String>,

    // --- paths ---
    /// Root data directory (`SPOOLMAN_DIR_DATA`).
    ///
    /// Defaults to `{user_data_dir}/spoolman`  (`~/.local/share/spoolman` on Linux,
    /// `%APPDATA%\spoolman` on Windows) — identical to upstream platformdirs.
    pub data_dir: PathBuf,
    /// Logs directory (`SPOOLMAN_DIR_LOGS`; defaults to `data_dir`).
    pub logs_dir: PathBuf,
    /// Backups directory (`SPOOLMAN_DIR_BACKUPS`; defaults to `data_dir/backups`).
    pub backups_dir: PathBuf,

    // --- server ---
    /// Listener address (`SPOOLMAN_HOST`; default `0.0.0.0`).
    pub host: String,
    /// Listener port (`SPOOLMAN_PORT`; default `7912`).
    pub port: u16,
    /// URL prefix for all routes (`SPOOLMAN_BASE_PATH`; default `""`).
    pub base_path: String,

    // --- features ---
    /// `SPOOLMAN_AUTOMATIC_BACKUP` (default `true`).
    pub automatic_backup: bool,
}

impl SpoolmanConfig {
    /// Build config from environment variables, mirroring upstream `env.py`.
    pub fn from_env() -> Result<Self> {
        let db_type = parse_db_type()?;
        let db_port = parse_optional_u16("SPOOLMAN_DB_PORT")?;
        let db_password = read_password()?;

        let data_dir = resolve_data_dir();
        let logs_dir = env::var("SPOOLMAN_DIR_LOGS")
            .map(PathBuf::from)
            .unwrap_or_else(|_| data_dir.clone());
        let backups_dir = env::var("SPOOLMAN_DIR_BACKUPS")
            .map(PathBuf::from)
            .unwrap_or_else(|_| data_dir.join("backups"));

        let port = parse_optional_u16("SPOOLMAN_PORT")?.unwrap_or(7912);

        let base_path = {
            let raw = env::var("SPOOLMAN_BASE_PATH").unwrap_or_default();
            if raw.is_empty() {
                String::new()
            } else {
                format!("/{}", raw.trim_matches('/'))
            }
        };

        let automatic_backup = parse_bool("SPOOLMAN_AUTOMATIC_BACKUP", true)?;

        Ok(SpoolmanConfig {
            db_type,
            db_host: env::var("SPOOLMAN_DB_HOST").ok(),
            db_port,
            db_name: env::var("SPOOLMAN_DB_NAME").ok(),
            db_username: env::var("SPOOLMAN_DB_USERNAME").ok(),
            db_password,
            data_dir,
            logs_dir,
            backups_dir,
            host: env::var("SPOOLMAN_HOST").unwrap_or_else(|_| "0.0.0.0".to_owned()),
            port,
            base_path,
            automatic_backup,
        })
    }

    /// Path to the SQLite database file — matches upstream's default location.
    pub fn sqlite_path(&self) -> PathBuf {
        self.data_dir.join("spoolman.db")
    }

    /// SQLite connection URL for sqlx.
    pub fn sqlite_url(&self) -> String {
        format!("sqlite://{}", self.sqlite_path().display())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn resolve_data_dir() -> PathBuf {
    env::var("SPOOLMAN_DIR_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("spoolman")
        })
}

fn parse_db_type() -> Result<DbType> {
    match env::var("SPOOLMAN_DB_TYPE").as_deref() {
        Ok("sqlite") | Err(_) => Ok(DbType::Sqlite),
        Ok(other) => Err(SpoolmanError::Config(format!(
            "SPOOLMAN_DB_TYPE='{other}' is not supported in this build; \
             only 'sqlite' is available until session 5 adds postgres/mysql"
        ))),
    }
}

fn parse_optional_u16(var: &str) -> Result<Option<u16>> {
    match env::var(var) {
        Err(_) => Ok(None),
        Ok(s) => s
            .parse::<u16>()
            .map(Some)
            .map_err(|e| SpoolmanError::Config(format!("Failed to parse {var}={s:?}: {e}"))),
    }
}

fn parse_bool(var: &str, default: bool) -> Result<bool> {
    match env::var(var).as_deref() {
        Err(_) => Ok(default),
        Ok("TRUE") | Ok("1") => Ok(true),
        Ok("FALSE") | Ok("0") => Ok(false),
        Ok(other) => Err(SpoolmanError::Config(format!(
            "Failed to parse {var}={other:?}: expected TRUE/FALSE/1/0"
        ))),
    }
}

/// Read the DB password from `SPOOLMAN_DB_PASSWORD_FILE` (preferred) or
/// `SPOOLMAN_DB_PASSWORD`, matching upstream `env.py::get_password()`.
fn read_password() -> Result<Option<String>> {
    if let Ok(file_path) = env::var("SPOOLMAN_DB_PASSWORD_FILE") {
        let content = fs::read_to_string(&file_path).map_err(|e| {
            SpoolmanError::Config(format!(
                "SPOOLMAN_DB_PASSWORD_FILE={file_path:?}: failed to read: {e}"
            ))
        })?;
        return Ok(Some(content));
    }
    Ok(env::var("SPOOLMAN_DB_PASSWORD").ok())
}
