//! Per-API-key MCP access (task 6.5): which tools a key may call, whether destructive
//! tools are enabled, and its rate limit. No row = [`McpPolicy::default`].

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use ts_rs::TS;

use crate::error::AppError;

pub const DEFAULT_RATE_PER_MINUTE: u32 = 60;
pub const MAX_RATE_PER_MINUTE: u32 = 600;

/// What one API key may do over MCP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct McpPolicy {
    /// Tool names the key may call; `None` = every read-only tool (the default).
    pub tools: Option<Vec<String>>,
    /// Destructive tools are refused unless this is on (and the tool is allowed).
    pub allow_destructive: bool,
    /// Requests per minute before `429`.
    pub rate_per_minute: u32,
}

impl Default for McpPolicy {
    fn default() -> Self {
        Self {
            tools: None,
            allow_destructive: false,
            rate_per_minute: DEFAULT_RATE_PER_MINUTE,
        }
    }
}

fn db_err(e: sqlx::Error) -> AppError {
    AppError::Database(e.to_string())
}

#[derive(sqlx::FromRow)]
struct Row {
    tools_json: Option<String>,
    allow_destructive: i64,
    rate_per_minute: i64,
}

/// The key's policy (defaults when none is stored).
pub async fn get(pool: &SqlitePool, api_key_id: i64) -> Result<McpPolicy, AppError> {
    let row: Option<Row> = sqlx::query_as(
        "SELECT tools_json, allow_destructive, rate_per_minute FROM mcp_key_policy \
         WHERE api_key_id = ?",
    )
    .bind(api_key_id)
    .fetch_optional(pool)
    .await
    .map_err(db_err)?;
    let Some(r) = row else {
        return Ok(McpPolicy::default());
    };
    let tools = match r.tools_json {
        Some(j) => Some(
            serde_json::from_str::<Vec<String>>(&j)
                .map_err(|e| AppError::Database(format!("mcp_key_policy.tools_json: {e}")))?,
        ),
        None => None,
    };
    Ok(McpPolicy {
        tools,
        allow_destructive: r.allow_destructive != 0,
        rate_per_minute: u32::try_from(r.rate_per_minute).unwrap_or(DEFAULT_RATE_PER_MINUTE),
    })
}

/// Store the key's policy (the caller validates tool names).
pub async fn set(pool: &SqlitePool, api_key_id: i64, p: &McpPolicy) -> Result<(), AppError> {
    if p.rate_per_minute == 0 || p.rate_per_minute > MAX_RATE_PER_MINUTE {
        return Err(AppError::InvalidInput(format!(
            "rate limit must be 1–{MAX_RATE_PER_MINUTE} requests per minute"
        )));
    }
    let tools_json = match &p.tools {
        Some(t) => Some(serde_json::to_string(t).map_err(|e| AppError::Internal(e.to_string()))?),
        None => None,
    };
    sqlx::query(
        "INSERT INTO mcp_key_policy (api_key_id, tools_json, allow_destructive, rate_per_minute, updated_at) \
         VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT(api_key_id) DO UPDATE SET tools_json = excluded.tools_json, \
           allow_destructive = excluded.allow_destructive, \
           rate_per_minute = excluded.rate_per_minute, updated_at = excluded.updated_at",
    )
    .bind(api_key_id)
    .bind(tools_json)
    .bind(i64::from(p.allow_destructive))
    .bind(i64::from(p.rate_per_minute))
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .map_err(db_err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn defaults_then_round_trip() {
        let dir = tempfile::tempdir().expect("dir");
        let db = crate::db::ForemanDb::connect(&dir.path().join("f.db"))
            .await
            .expect("db");
        let (key, _) = crate::auth::create_api_key(&db.pool, "Claude")
            .await
            .expect("key");
        assert_eq!(
            get(&db.pool, key.id).await.expect("get"),
            McpPolicy::default()
        );
        let p = McpPolicy {
            tools: Some(vec!["list_printers".into(), "pause_print".into()]),
            allow_destructive: true,
            rate_per_minute: 10,
        };
        set(&db.pool, key.id, &p).await.expect("set");
        assert_eq!(get(&db.pool, key.id).await.expect("get"), p);
        let bad = McpPolicy {
            rate_per_minute: 0,
            ..McpPolicy::default()
        };
        assert!(set(&db.pool, key.id, &bad).await.is_err());
    }
}
