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

use crate::db::Engine;

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
pub fn parse_filter_terms(raw: &str) -> Vec<FilterTerm> {
    if raw.is_empty() {
        return vec![FilterTerm::IsNull];
    }
    raw.split(',')
        .map(|t| {
            let t = t.trim();
            if t.is_empty() {
                FilterTerm::IsNull
            } else if t.starts_with('"') && t.ends_with('"') && t.len() >= 2 {
                FilterTerm::Exact(t[1..t.len() - 1].to_owned())
            } else {
                FilterTerm::Fuzzy(t.to_owned())
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

    /// Add a string filter.  Empty `raw` ⇒ IS NULL; comma-list ⇒ OR of terms.
    pub fn add_str(&mut self, column: &str, raw: &str, engine: &Engine) {
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
            if id.is_empty() {
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

    /// Add a datetime range filter.  `range` is `"start|end"`, `"start|"`, `"|end"`.
    pub fn add_datetime_range_opt(&mut self, column: &str, range: Option<&str>) {
        let Some(range) = range else { return };
        let parts: Vec<&str> = range.splitn(2, '|').collect();
        let start = parts.first().copied().unwrap_or("").trim();
        let end = parts.get(1).copied().unwrap_or("").trim();
        if !start.is_empty() {
            let n = self.next_placeholder();
            self.binds.push(start.to_owned());
            self.clauses.push(format!("{column} >= {n}"));
        }
        if !end.is_empty() {
            let n = self.next_placeholder();
            self.binds.push(end.to_owned());
            self.clauses.push(format!("{column} <= {n}"));
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
                FilterTerm::IsNull => parts.push(format!("{column} IS NULL")),
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
    fn order_by_nulls_last() {
        let clauses = parse_sort(Some("name:asc,registered:desc"), "id:asc");
        let sql = order_by_sql(&clauses);
        assert!(sql.contains("(name IS NULL) ASC, name ASC"));
        assert!(sql.contains("(registered IS NULL) ASC, registered DESC"));
    }
}
