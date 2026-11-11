//! Repository for the `shell_macro` table.
//!
//! Shell macros are named multi-line SSH commands.  Scope `"printer"` means the macro
//! is shown only on a specific printer's terminal; `"all_ssh"` means it appears on
//! every printer that has SSH configured.

use chrono::Utc;
use sqlx::{FromRow, SqlitePool};

use crate::{
    domain::{CreateShellMacro, ShellMacro, UpdateShellMacro},
    error::AppError,
};

// ---------------------------------------------------------------------------
// Row type
// ---------------------------------------------------------------------------

#[derive(FromRow)]
struct MacroRow {
    id: i64,
    name: String,
    commands: String,
    scope: String,
    printer_id: Option<i64>,
    ask_before_run: i64, // 0/1 for bool
    created_at: String,
    updated_at: String,
}

fn parse_dt(s: &str) -> chrono::DateTime<Utc> {
    s.parse().unwrap_or_else(|_| Utc::now())
}

impl MacroRow {
    fn into_domain(self) -> ShellMacro {
        ShellMacro {
            id: self.id,
            name: self.name,
            commands: self.commands,
            scope: self.scope,
            printer_id: self.printer_id,
            ask_before_run: self.ask_before_run != 0,
            created_at: parse_dt(&self.created_at),
            updated_at: parse_dt(&self.updated_at),
        }
    }
}

// ---------------------------------------------------------------------------
// Auto-detect ask_before_run
// ---------------------------------------------------------------------------

/// Returns `true` if the commands contain one of `sudo|rm|reboot|shutdown|dd|mkfs` as a
/// whole word (same rule as the UI: `\b(sudo|rm|reboot|shutdown|dd|mkfs)\b`).
pub fn should_ask(commands: &str) -> bool {
    commands
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|w| matches!(w, "sudo" | "rm" | "reboot" | "shutdown" | "dd" | "mkfs"))
}

fn check_scope(scope: &str) -> Result<(), AppError> {
    if scope == "printer" || scope == "all_ssh" {
        Ok(())
    } else {
        Err(AppError::InvalidInput(format!(
            "scope must be 'printer' or 'all_ssh', got '{scope}'"
        )))
    }
}

// ---------------------------------------------------------------------------
// CRUD
// ---------------------------------------------------------------------------

const SQL_GET: &str =
    "SELECT id, name, commands, scope, printer_id, ask_before_run, created_at, updated_at \
     FROM shell_macro WHERE id = ?";

/// Return a single macro by id.
pub async fn get(pool: &SqlitePool, id: i64) -> Result<ShellMacro, AppError> {
    let row: Option<MacroRow> = sqlx::query_as(SQL_GET)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;
    row.map(MacroRow::into_domain)
        .ok_or_else(|| AppError::NotFound(format!("shell_macro {id}")))
}

/// List macros visible on a given printer:
/// - all macros where `scope = 'all_ssh'`
/// - all macros where `scope = 'printer'` AND `printer_id = ?`
pub async fn list_for_printer(
    pool: &SqlitePool,
    printer_id: i64,
) -> Result<Vec<ShellMacro>, AppError> {
    let rows: Vec<MacroRow> = sqlx::query_as(
        "SELECT id, name, commands, scope, printer_id, ask_before_run, created_at, updated_at \
         FROM shell_macro \
         WHERE scope = 'all_ssh' OR (scope = 'printer' AND printer_id = ?) \
         ORDER BY name ASC",
    )
    .bind(printer_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Database(e.to_string()))?;

    Ok(rows.into_iter().map(MacroRow::into_domain).collect())
}

/// Create a new shell macro.
///
/// If `ask_before_run` is not set and the commands body contains dangerous
/// keywords, `ask_before_run` is automatically set to `true`.
pub async fn create(
    pool: &SqlitePool,
    printer_id: i64,
    req: CreateShellMacro,
) -> Result<ShellMacro, AppError> {
    let scope = req.scope.as_deref().unwrap_or("printer").to_owned();
    check_scope(&scope)?;
    if req.name.trim().is_empty() || req.commands.trim().is_empty() {
        return Err(AppError::InvalidInput(
            "name and command are required".into(),
        ));
    }
    let macro_printer_id = if scope == "all_ssh" {
        None
    } else {
        Some(printer_id)
    };
    // Dangerous commands always ask, even if the client sent false.
    let ask = req.ask_before_run.unwrap_or(false) || should_ask(&req.commands);
    let now = Utc::now().to_rfc3339();

    let id = sqlx::query(
        "INSERT INTO shell_macro \
           (name, commands, scope, printer_id, ask_before_run, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&req.name)
    .bind(&req.commands)
    .bind(&scope)
    .bind(macro_printer_id)
    .bind(ask as i64)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| AppError::Database(e.to_string()))?
    .last_insert_rowid();

    get(pool, id).await
}

/// Update fields on an existing shell macro.  `None` fields are left unchanged.
/// `printer_id` is used when the scope changes to `"printer"`.
pub async fn update(
    pool: &SqlitePool,
    printer_id: i64,
    id: i64,
    req: UpdateShellMacro,
) -> Result<ShellMacro, AppError> {
    let current = get(pool, id).await?;
    let name = req.name.unwrap_or(current.name);
    let commands = req.commands.unwrap_or_else(|| current.commands.clone());
    let scope = req.scope.unwrap_or(current.scope);
    check_scope(&scope)?;
    let macro_printer_id = if scope == "all_ssh" {
        None
    } else {
        current.printer_id.or(Some(printer_id))
    };
    let ask = req.ask_before_run.unwrap_or(current.ask_before_run) || should_ask(&commands);
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "UPDATE shell_macro SET name = ?, commands = ?, scope = ?, printer_id = ?, \
         ask_before_run = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&name)
    .bind(&commands)
    .bind(&scope)
    .bind(macro_printer_id)
    .bind(ask as i64)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| AppError::Database(e.to_string()))?;

    get(pool, id).await
}

/// Delete a shell macro.
pub async fn delete(pool: &SqlitePool, id: i64) -> Result<(), AppError> {
    let n = sqlx::query("DELETE FROM shell_macro WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?
        .rows_affected();
    if n == 0 {
        Err(AppError::NotFound(format!("shell_macro {id}")))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::ForemanDb,
        domain::{CreatePrinter, FirmwareKind},
        repo::printer,
    };
    use tempfile::TempDir;

    async fn db() -> (ForemanDb, TempDir) {
        let dir = TempDir::new().expect("tempdir");
        let db = ForemanDb::connect(&dir.path().join("foreman.db"))
            .await
            .expect("connect");
        (db, dir)
    }

    async fn create_printer(pool: &SqlitePool) -> i64 {
        printer::create(
            pool,
            CreatePrinter {
                name: "TestPrinter".to_owned(),
                firmware: FirmwareKind::Moonraker,
                primary_url: None,
                api_key: None,
                serial_number: None,
                access_code: None,
                tags: vec![],
                hardened_nozzle: false,
                slicer_profiles: vec![],
                sort_order: None,
                fail_after_checks: None,
                return_after_secs: None,
            },
        )
        .await
        .expect("create printer")
        .id
    }

    #[tokio::test]
    async fn create_and_list_macro() {
        let (db, _dir) = db().await;
        let pid = create_printer(&db.pool).await;

        let m = create(
            &db.pool,
            pid,
            CreateShellMacro {
                name: "restart cam".to_owned(),
                commands: "sudo systemctl restart crowsnest".to_owned(),
                scope: Some("printer".to_owned()),
                ask_before_run: None, // should auto-detect → true (contains sudo)
            },
        )
        .await
        .expect("create");

        assert_eq!(m.name, "restart cam");
        assert!(
            m.ask_before_run,
            "sudo command should auto-set ask_before_run"
        );
        assert_eq!(m.printer_id, Some(pid));

        let list = list_for_printer(&db.pool, pid).await.expect("list");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "restart cam");
    }

    #[tokio::test]
    async fn update_macro() {
        let (db, _dir) = db().await;
        let pid = create_printer(&db.pool).await;

        let m = create(
            &db.pool,
            pid,
            CreateShellMacro {
                name: "uptime".to_owned(),
                commands: "uptime".to_owned(),
                scope: None,
                ask_before_run: Some(false),
            },
        )
        .await
        .expect("create");

        let updated = update(
            &db.pool,
            pid,
            m.id,
            UpdateShellMacro {
                name: Some("check uptime".to_owned()),
                commands: None,
                scope: Some("all_ssh".to_owned()),
                ask_before_run: None,
            },
        )
        .await
        .expect("update");

        assert_eq!(updated.name, "check uptime");
        assert!(!updated.ask_before_run);
        assert_eq!(updated.scope, "all_ssh");
        assert_eq!(updated.printer_id, None);

        // Client can't switch ask off for a dangerous command.
        let updated = update(
            &db.pool,
            pid,
            m.id,
            UpdateShellMacro {
                name: None,
                commands: Some("sudo reboot".to_owned()),
                scope: None,
                ask_before_run: Some(false),
            },
        )
        .await
        .expect("update 2");
        assert!(updated.ask_before_run);
    }

    #[tokio::test]
    async fn all_ssh_scope_visible_on_any_printer() {
        let (db, _dir) = db().await;
        let pid1 = create_printer(&db.pool).await;
        let pid2 = create_printer(&db.pool).await;

        create(
            &db.pool,
            pid1,
            CreateShellMacro {
                name: "global".to_owned(),
                commands: "echo hello".to_owned(),
                scope: Some("all_ssh".to_owned()),
                ask_before_run: Some(false),
            },
        )
        .await
        .expect("create");

        let list1 = list_for_printer(&db.pool, pid1).await.expect("list1");
        let list2 = list_for_printer(&db.pool, pid2).await.expect("list2");
        assert_eq!(list1.len(), 1);
        assert_eq!(list2.len(), 1);
        assert_eq!(list1[0].name, "global");
    }

    #[test]
    fn sudo_triggers_ask_before_run() {
        assert!(should_ask("sudo systemctl restart crowsnest"));
        assert!(should_ask("rm /tmp/file.txt"));
        assert!(should_ask("reboot"));
        assert!(should_ask("dd if=/dev/zero of=x"));
        assert!(should_ask("mkfs.ext4 /dev/sda1"));
        assert!(!should_ask("uptime"));
        assert!(!should_ask("ls /home/pi"));
        assert!(!should_ask("git add . && echo firmware"));
    }
}
