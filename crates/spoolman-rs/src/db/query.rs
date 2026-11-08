//! Filter, sort, and pagination helpers for the spoolman-rs query layer.
//!
//! Ported from `vendor/spoolman-upstream/spoolman/database/utils.py`.
//!
//! ## Filter semantics (upstream)
//! A filter value is a comma-separated list of *terms*.
//! - `"term"` (quoted)   → exact match (`field = 'term'`)
//! - `term`  (unquoted)  → case-insensitive fuzzy (`ILIKE '%term%'` on Pg/MySQL; `LIKE` on SQLite)
//! - `` (empty string)   → `field IS NULL`
//!
//! ## Sort semantics
//! `sort` query param: comma-separated `field:asc` / `field:desc` pairs.
//! NULLs always last: `(field IS NULL) ASC, field DIR`.
//!
//! ## Pagination
//! `limit` / `offset` with `x-total-count` response header.

use std::fmt::Write;
use std::sync::OnceLock;

use crate::db::{vendor::normalise_dt_bound, Engine};

// ---------------------------------------------------------------------------
// Placeholder translation — `?` → `$N` for PostgreSQL
// ---------------------------------------------------------------------------
//
// sqlx 0.9's AnyPool does not automatically translate `?` parameter markers to
// the engine-native `$N` form when the backend is PostgreSQL.  All queries in
// this crate use `?`, which is understood by SQLite and MySQL but NOT Postgres.
//
// Rather than threading `engine` through every function, we store it in a
// module-global OnceLock (set once during Db::connect, never changes) and
// expose `tp()` which rewrites `?` → `$1`, `$2`, … when running on Postgres.

static GLOBAL_ENGINE: OnceLock<Engine> = OnceLock::new();

/// Called once from `Db::connect` to register the engine dialect.
pub fn set_global_engine(engine: Engine) {
    GLOBAL_ENGINE.set(engine).ok(); // first caller wins; idempotent
}

/// Placeholder expression for a datetime bind parameter.
///
/// SQLite stores datetimes as TEXT, so a bare `?` suffices.
/// PostgreSQL requires an explicit cast to `TIMESTAMP WITHOUT TIME ZONE`.
/// Use this inside `format!()` for datetime columns, then wrap with `tp()`.
pub fn dtp() -> &'static str {
    match GLOBAL_ENGINE.get() {
        Some(Engine::Postgres) => "?::TIMESTAMP",
        _ => "?",
    }
}

/// Translate `?` placeholders to `$N` when the backend is PostgreSQL.
///
/// Returns the input unchanged for SQLite/MySQL.  Cheap no-op when the SQL
/// contains no `?` at all.
pub fn tp(sql: &str) -> String {
    match GLOBAL_ENGINE.get() {
        Some(Engine::Postgres) if sql.contains('?') => {
            let mut out = String::with_capacity(sql.len() + 32);
            let mut n = 0u32;
            for ch in sql.chars() {
                if ch == '?' {
                    n += 1;
                    write!(out, "${n}").unwrap();
                } else {
                    out.push(ch);
                }
            }
            out
        }
        _ => sql.to_owned(),
    }
}

#[allow(unused_imports)]
use chrono; // used by validate_datetime_filter

// ---------------------------------------------------------------------------
// Filter terms
// ---------------------------------------------------------------------------

/// One parsed term from a comma-separated filter value.
#[derive(Debug, Clone)]
pub enum FilterTerm {
    /// `"exact"` — quoted in the query string.
    Exact(String),
    /// `fuzzy` — unquoted.
    Fuzzy(String),
    /// `` — empty string means IS NULL.
    IsNull,
}

/// Parse a raw filter value (e.g. `"PLA","PETG",ABS`) into terms.
///
/// Splits on commas, but commas inside double-quoted strings are preserved
/// (e.g. `"Top shelf, Rack 3"` is a single exact term).
pub fn parse_filter_terms(raw: &str) -> Vec<FilterTerm> {
    if raw.is_empty() {
        return vec![FilterTerm::IsNull];
    }

    // Quote-aware split: commas inside "..." are not term separators.
    let mut terms = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in raw.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                current.push(ch);
            }
            ',' if !in_quotes => {
                terms.push(current.trim().to_owned());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    terms.push(current.trim().to_owned());

    terms
        .into_iter()
        .map(|t| {
            if t.is_empty() {
                FilterTerm::IsNull
            } else if t.starts_with('"') && t.ends_with('"') && t.len() >= 2 {
                FilterTerm::Exact(t[1..t.len() - 1].to_owned())
            } else {
                FilterTerm::Fuzzy(t)
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Building SQL WHERE fragments and ORDER BY clauses
// ---------------------------------------------------------------------------

/// A SQL fragment builder that accumulates WHERE conditions and binds.
///
/// We cannot use `sqlx::QueryBuilder` here because we need to build fragments
/// that are shared across a data-query and a count-query on the same filter set.
/// Instead we produce `(sql_fragment, binds)` pairs and let the callers weave
/// them into their own QueryBuilders.
#[derive(Default)]
pub struct WhereBuilder {
    clauses: Vec<String>,
    /// Values to bind, in order, as strings.  The caller re-binds them.
    pub binds: Vec<String>,
}

impl WhereBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// True when at least one condition has been added.
    pub fn has_conditions(&self) -> bool {
        !self.clauses.is_empty()
    }

    /// Push a pre-built SQL clause (no binds needed from it).
    pub fn push_raw_clause(&mut self, clause: String) {
        if !clause.is_empty() {
            self.clauses.push(clause);
        }
    }

    /// Return the WHERE keyword + conditions as a single string, or empty.
    pub fn sql(&self) -> String {
        if self.clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", self.clauses.join(" AND "))
        }
    }

    /// Add an optional string filter on `column` for the given engine.
    ///
    /// `None` → skipped; `Some(raw)` → parsed and appended.
    pub fn add_str_opt(&mut self, column: &str, raw: Option<&str>, engine: &Engine) {
        if let Some(r) = raw {
            self.add_str(column, r, engine);
        }
    }

    /// Add a string filter for a **nullable** column.
    /// Empty `raw` ⇒ `col IS NULL OR col = ''`; comma-list ⇒ OR of terms.
    pub fn add_str(&mut self, column: &str, raw: &str, engine: &Engine) {
        let terms = parse_filter_terms(raw);
        self.append_terms(column, &terms, engine);
    }

    /// Add an optional string filter for a **non-nullable** column (e.g. `vendor.name`).
    ///
    /// `None` → skipped; empty `raw` ⇒ `col = ''` (not IS NULL); otherwise same as `add_str`.
    pub fn add_str_required_opt(&mut self, column: &str, raw: Option<&str>, engine: &Engine) {
        if let Some(r) = raw {
            self.add_str_required(column, r, engine);
        }
    }

    /// Add a string filter for a **non-nullable** column.
    /// Empty `raw` ⇒ `col = ''`; comma-list ⇒ OR of terms (same as `add_str` for non-empty).
    pub fn add_str_required(&mut self, column: &str, raw: &str, engine: &Engine) {
        if raw.is_empty() {
            let n = self.next_placeholder();
            self.binds.push(String::new());
            self.clauses.push(format!("{column} = {n}"));
            return;
        }
        let terms = parse_filter_terms(raw);
        self.append_terms(column, &terms, engine);
    }

    /// Add an optional integer filter (comma-list of exact IDs).
    pub fn add_int_opt(&mut self, column: &str, raw: Option<&str>) {
        if let Some(r) = raw {
            self.add_int(column, r);
        }
    }

    /// Add an integer filter.  Empty ⇒ IS NULL; comma-list ⇒ OR of exact matches.
    pub fn add_int(&mut self, column: &str, raw: &str) {
        if raw.is_empty() {
            self.clauses.push(format!("{column} IS NULL"));
            return;
        }
        let ids: Vec<&str> = raw.split(',').map(str::trim).collect();
        let mut parts = Vec::with_capacity(ids.len());
        for id in ids {
            if id.is_empty() || id == "-1" {
                parts.push(format!("{column} IS NULL"));
            } else {
                let n = self.next_placeholder();
                self.binds.push(id.to_owned());
                parts.push(format!("{column} = {n}"));
            }
        }
        self.clauses.push(format!("({})", parts.join(" OR ")));
    }

    /// Add a boolean filter (e.g. archived).
    pub fn add_bool(&mut self, column: &str, value: bool) {
        // Cross-engine: 1/0 works on all three.
        let n = self.next_placeholder();
        self.binds.push(if value {
            "1".to_owned()
        } else {
            "0".to_owned()
        });
        self.clauses.push(format!("{column} = {n}"));
    }

    /// Add a datetime filter.
    ///
    /// Grammar (mirrors upstream `add_where_clause_datetime_opt`):
    /// - `""` → `col IS NULL`
    /// - `"<ts>"` → exact match `col = ?`
    /// - `"<start>|<end>"` → inclusive range (either end may be empty to leave it open)
    /// - Comma-separated list of the above → OR of conditions
    pub fn add_datetime_range_opt(&mut self, column: &str, raw: Option<&str>) {
        let Some(raw) = raw else { return };

        let mut parts: Vec<String> = Vec::new();
        for segment in raw.split(',') {
            if segment.is_empty() {
                // Empty segment → IS NULL
                parts.push(format!("{column} IS NULL"));
                continue;
            }
            if let Some(pipe_pos) = segment.find('|') {
                // Range: start|end — convert bounds to UTC for consistent string comparison.
                let start = segment[..pipe_pos].trim();
                let end = segment[pipe_pos + 1..].trim();
                let mut bounds = Vec::new();
                if !start.is_empty() {
                    let n = self.next_placeholder();
                    self.binds.push(normalise_dt_bound(start));
                    bounds.push(format!("{column} >= {n}"));
                }
                if !end.is_empty() {
                    let n = self.next_placeholder();
                    self.binds.push(normalise_dt_bound(end));
                    bounds.push(format!("{column} <= {n}"));
                }
                if !bounds.is_empty() {
                    parts.push(format!("({})", bounds.join(" AND ")));
                }
            } else {
                // Exact match (no pipe) — convert to UTC.
                let ts = segment.trim();
                if !ts.is_empty() {
                    let n = self.next_placeholder();
                    self.binds.push(normalise_dt_bound(ts));
                    parts.push(format!("{column} = {n}"));
                }
            }
        }

        if !parts.is_empty() {
            self.clauses.push(format!("({})", parts.join(" OR ")));
        }
    }

    // --- internals ---

    fn next_placeholder(&self) -> String {
        // AnyPool normalises all engines to `?`.
        "?".to_owned()
    }

    fn append_terms(&mut self, column: &str, terms: &[FilterTerm], engine: &Engine) {
        if terms.is_empty() {
            return;
        }
        let mut parts = Vec::with_capacity(terms.len());
        for term in terms {
            match term {
                FilterTerm::IsNull => {
                    let n = self.next_placeholder();
                    self.binds.push(String::new());
                    parts.push(format!("({column} IS NULL OR {column} = {n})"));
                }
                FilterTerm::Exact(v) => {
                    let n = self.next_placeholder();
                    self.binds.push(v.clone());
                    parts.push(format!("{column} = {n}"));
                }
                FilterTerm::Fuzzy(v) => {
                    let n = self.next_placeholder();
                    // SQLite LIKE is case-insensitive for ASCII by default.
                    // Postgres/MySQL use ILIKE / LIKE (MySQL LIKE is case-insensitive
                    // on case-insensitive collations, which is the default).
                    let op = match engine {
                        Engine::Postgres => "ILIKE",
                        _ => "LIKE",
                    };
                    self.binds.push(format!("%{v}%"));
                    parts.push(format!("{column} {op} {n}"));
                }
            }
        }
        self.clauses.push(format!("({})", parts.join(" OR ")));
    }
}

// ---------------------------------------------------------------------------
// ORDER BY
// ---------------------------------------------------------------------------

/// Parsed sort clause.
#[derive(Debug, Clone)]
pub struct SortClause {
    pub column: String,
    pub ascending: bool,
}

/// Parse `sort` query param: `"field:asc,other:desc"`.
pub fn parse_sort(raw: Option<&str>, default: &str) -> Vec<SortClause> {
    let s = raw.unwrap_or(default);
    s.split(',')
        .filter_map(|part| {
            let mut it = part.trim().splitn(2, ':');
            let col = it.next()?.trim().to_owned();
            if col.is_empty() {
                return None;
            }
            let dir = it.next().unwrap_or("asc").trim().to_lowercase();
            Some(SortClause {
                column: col,
                ascending: dir != "desc",
            })
        })
        .collect()
}

/// Qualify sort column names using a lookup table.
///
/// `qualifications` maps bare names (e.g. `"id"`) to qualified names
/// (e.g. `"s.id"`).  Columns not found in the map are left as-is.
pub fn qualify_sort(clauses: &[SortClause], qualifications: &[(&str, &str)]) -> Vec<SortClause> {
    clauses
        .iter()
        .map(|c| {
            let column = qualifications
                .iter()
                .find(|(bare, _)| *bare == c.column)
                .map(|(_, qualified)| qualified.to_string())
                .unwrap_or_else(|| c.column.clone());
            SortClause {
                column,
                ascending: c.ascending,
            }
        })
        .collect()
}

/// Build an ORDER BY clause with NULLs-last semantics (all engines).
///
/// For each field: `(field IS NULL) ASC, field DIR`.
pub fn order_by_sql(clauses: &[SortClause]) -> String {
    if clauses.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = clauses
        .iter()
        .map(|c| {
            let dir = if c.ascending { "ASC" } else { "DESC" };
            format!("({col} IS NULL) ASC, {col} {dir}", col = c.column)
        })
        .collect();
    format!("ORDER BY {}", parts.join(", "))
}

// ---------------------------------------------------------------------------
// Datetime filter validation
// ---------------------------------------------------------------------------

/// Parse a single datetime string and return Ok(()) or Err(message).
///
/// Accepts RFC 3339 (with Z or ±HH:MM offset) and also ISO 8601 naive datetimes
/// (treated as UTC by upstream).
fn parse_dt_bound(raw: &str) -> Result<(), String> {
    let s = raw.trim();
    if s.is_empty() {
        return Ok(());
    }
    // Try RFC 3339 first (handles "Z" and "+HH:MM").
    if chrono::DateTime::parse_from_rfc3339(s).is_ok() {
        return Ok(());
    }
    // Try naive datetime (no timezone, treated as UTC upstream).
    if chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").is_ok() {
        return Ok(());
    }
    if chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f").is_ok() {
        return Ok(());
    }
    Err(format!("Invalid datetime value: '{s}'"))
}

/// Validate a datetime filter string (comma-list of exact-or-range terms).
///
/// Returns `Err(message)` if any term cannot be parsed as a datetime.
/// Returns `Ok(())` if the entire filter is syntactically valid.
pub fn validate_datetime_filter(raw: &str) -> Result<(), String> {
    for segment in raw.split(',') {
        if segment.is_empty() {
            continue; // IS NULL — valid
        }
        if let Some(pipe_pos) = segment.find('|') {
            let start = segment[..pipe_pos].trim();
            let end = segment[pipe_pos + 1..].trim();
            if start.is_empty() && end.is_empty() {
                return Err(
                    "Empty datetime range '|': at least one bound must be non-empty.".into(),
                );
            }
            if !start.is_empty() {
                parse_dt_bound(start)?;
            }
            if !end.is_empty() {
                parse_dt_bound(end)?;
            }
        } else {
            parse_dt_bound(segment)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Pagination
// ---------------------------------------------------------------------------

/// Extracted limit/offset from query params.
#[derive(Debug, Clone, Copy)]
pub struct Pagination {
    pub limit: i64,
    pub offset: i64,
}

impl Pagination {
    pub fn new(limit: Option<i64>, offset: Option<i64>) -> Self {
        Self {
            limit: limit.unwrap_or(100).clamp(1, 10_000),
            offset: offset.unwrap_or(0).max(0),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_terms_empty_is_null() {
        let terms = parse_filter_terms("");
        assert!(matches!(terms.as_slice(), [FilterTerm::IsNull]));
    }

    #[test]
    fn filter_terms_quoted_exact() {
        let terms = parse_filter_terms("\"PLA\"");
        assert!(matches!(terms.as_slice(), [FilterTerm::Exact(s)] if s == "PLA"));
    }

    #[test]
    fn filter_terms_unquoted_fuzzy() {
        let terms = parse_filter_terms("PETG");
        assert!(matches!(terms.as_slice(), [FilterTerm::Fuzzy(s)] if s == "PETG"));
    }

    #[test]
    fn filter_terms_mixed() {
        let terms = parse_filter_terms("\"PLA\",PETG,");
        assert_eq!(terms.len(), 3);
        assert!(matches!(&terms[0], FilterTerm::Exact(s) if s == "PLA"));
        assert!(matches!(&terms[1], FilterTerm::Fuzzy(s) if s == "PETG"));
        assert!(matches!(&terms[2], FilterTerm::IsNull));
    }

    #[test]
    fn filter_terms_quoted_with_comma() {
        let terms = parse_filter_terms("\"Top shelf, Rack 3\"");
        assert_eq!(terms.len(), 1);
        assert!(matches!(&terms[0], FilterTerm::Exact(s) if s == "Top shelf, Rack 3"));
    }

    #[test]
    fn filter_terms_quoted_comma_and_unquoted() {
        let terms = parse_filter_terms("\"Top shelf, Rack 3\",Garage");
        assert_eq!(terms.len(), 2);
        assert!(matches!(&terms[0], FilterTerm::Exact(s) if s == "Top shelf, Rack 3"));
        assert!(matches!(&terms[1], FilterTerm::Fuzzy(s) if s == "Garage"));
    }

    #[test]
    fn order_by_nulls_last() {
        let clauses = parse_sort(Some("name:asc,registered:desc"), "id:asc");
        let sql = order_by_sql(&clauses);
        assert!(sql.contains("(name IS NULL) ASC, name ASC"));
        assert!(sql.contains("(registered IS NULL) ASC, registered DESC"));
    }
}
