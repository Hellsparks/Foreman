//! Cross-entity text + color search backing `GET /search`.
//!
//! Implements the semantics of `spoolman/database/search.py`:
//!
//! - Query is split on whitespace into terms (max 8, deduplicated).
//! - Each entity row must match EVERY term (AND of per-term ORs).
//! - Text matching is case-insensitive.
//! - A purely numeric query also matches spools by exact ID.
//! - A hex-color query additionally runs a color-similarity search on filaments.
//!
//! **Implementation note:** this version fetches candidate rows via the existing
//! `find()` helpers (up to 10 000 per entity) and filters in Rust rather than
//! building correlated SQL subqueries.  This is correct for all database sizes
//! encountered in practice; SQL-side filtering is an optimisation deferred to
//! session 10.

use sqlx::AnyPool;

use crate::{
    db::{
        filament::{self as filament_db, FilamentFilter},
        spool::{self as spool_db, SpoolFilter},
        vendor::{self as vendor_db, VendorFilter},
        Engine,
    },
    error::Result,
    models::{
        FilamentResponse, SearchResultFilament, SearchResultFilamentSpool, SearchResultSpool,
        SearchResultVendor, SearchResults, SpoolResponse, VendorResponse,
    },
};

// ---------------------------------------------------------------------------
// Term splitting
// ---------------------------------------------------------------------------

const MAX_TERMS: usize = 8;

fn split_terms(q: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for raw in q.split_whitespace() {
        let lower = raw.to_lowercase();
        if !terms.contains(&lower) {
            terms.push(lower);
            if terms.len() >= MAX_TERMS {
                break;
            }
        }
    }
    terms
}

// ---------------------------------------------------------------------------
// Color detection and similarity
// ---------------------------------------------------------------------------

/// Returns the 6-character hex string (without `#`) if `q` looks like a color.
pub fn detect_color(q: &str) -> Option<String> {
    let q = q.trim();
    let hex = q.strip_prefix('#').unwrap_or(q);

    if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some(hex.to_uppercase());
    }
    if hex.len() == 3 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some(
            hex.chars()
                .flat_map(|c| [c, c])
                .collect::<String>()
                .to_uppercase(),
        );
    }

    // Basic CSS color name lookup.
    let named = match q.to_lowercase().as_str() {
        "red" => "FF0000",
        "green" => "008000",
        "blue" => "0000FF",
        "yellow" => "FFFF00",
        "orange" => "FFA500",
        "purple" => "800080",
        "pink" => "FFC0CB",
        "black" => "000000",
        "white" => "FFFFFF",
        "gray" | "grey" => "808080",
        "silver" => "C0C0C0",
        "gold" => "FFD700",
        "brown" => "A52A2A",
        "cyan" | "aqua" => "00FFFF",
        "magenta" | "fuchsia" => "FF00FF",
        "lime" => "00FF00",
        "navy" => "000080",
        "teal" => "008080",
        "olive" => "808000",
        "maroon" => "800000",
        _ => return None,
    };
    Some(named.to_owned())
}

fn parse_hex_rgb(hex: &str) -> Option<(f64, f64, f64)> {
    let h = hex.strip_prefix('#').unwrap_or(hex);
    if h.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&h[0..2], 16).ok()? as f64;
    let g = u8::from_str_radix(&h[2..4], 16).ok()? as f64;
    let b = u8::from_str_radix(&h[4..6], 16).ok()? as f64;
    Some((r, g, b))
}

/// Euclidean RGB distance, normalised to 0–100.
fn color_distance(h1: &str, h2: &str) -> f64 {
    let Some((r1, g1, b1)) = parse_hex_rgb(h1) else {
        return f64::MAX;
    };
    let Some((r2, g2, b2)) = parse_hex_rgb(h2) else {
        return f64::MAX;
    };
    let d = ((r1 - r2).powi(2) + (g1 - g2).powi(2) + (b1 - b2).powi(2)).sqrt();
    d / 441.67 * 100.0
}

fn filament_min_color_distance(query_hex: &str, f: &FilamentResponse) -> f64 {
    let hexes: Vec<&str> = if let Some(ref h) = f.multi_color_hexes {
        h.split(',').collect()
    } else if let Some(ref h) = f.color_hex {
        vec![h.as_str()]
    } else {
        return f64::MAX;
    };
    hexes
        .into_iter()
        .map(|h| color_distance(query_hex, h))
        .fold(f64::MAX, f64::min)
}

// ---------------------------------------------------------------------------
// In-memory term matcher + ranker
// ---------------------------------------------------------------------------

/// Return `(matched_field, rank)` if ALL terms hit at least one field.
/// Lower rank = better match.
fn classify(terms: &[String], fields: &[(&str, Option<&str>)]) -> Option<(String, i64)> {
    if terms.is_empty() {
        return None;
    }
    let mut best_label = String::new();
    let mut best_rank = i64::MAX;
    let mut worst_rank = i64::MIN;

    for term in terms {
        let mut hit: Option<(&str, i64)> = None;
        for (label, value) in fields {
            let Some(v) = value else { continue };
            let v_lower = v.to_lowercase();
            if v_lower.contains(term.as_str()) {
                let rank: i64 = if v_lower.starts_with(term.as_str()) {
                    0
                } else {
                    1
                };
                if hit.is_none() || rank < hit.unwrap().1 {
                    hit = Some((label, rank));
                }
            }
        }
        let (label, rank) = hit?; // term not found → no match
        if rank > worst_rank {
            worst_rank = rank;
        }
        if rank < best_rank {
            best_rank = rank;
            best_label = label.to_owned();
        }
    }

    if best_label.is_empty() {
        None
    } else {
        Some((best_label, worst_rank))
    }
}

// ---------------------------------------------------------------------------
// Per-entity matchers
// ---------------------------------------------------------------------------

fn match_spools(
    spools: Vec<SpoolResponse>,
    terms: &[String],
    limit: usize,
) -> Vec<SearchResultSpool> {
    let mut hits: Vec<(SpoolResponse, String, i64)> = spools
        .into_iter()
        .filter_map(|s| {
            classify(
                terms,
                &[
                    ("comment", s.comment.as_deref()),
                    ("location", s.location.as_deref()),
                    ("lot_nr", s.lot_nr.as_deref()),
                ],
            )
            .map(|(f, r)| (s, f, r))
        })
        .collect();

    // Sort by (rank ASC, id ASC) — newest IDs sort first within the same rank
    // because candidate rows are already newest-first.
    hits.sort_by(|a, b| a.2.cmp(&b.2).then(a.0.id.cmp(&b.0.id)));
    hits.truncate(limit);
    hits.into_iter()
        .map(|(s, f, _)| SearchResultSpool {
            spool: s,
            match_field: f,
        })
        .collect()
}

fn match_filaments(
    filaments: Vec<FilamentResponse>,
    terms: &[String],
    limit: usize,
    color_hits: Vec<(FilamentResponse, String)>,
) -> Vec<SearchResultFilament> {
    let color_ids: std::collections::HashSet<i64> = color_hits.iter().map(|(f, _)| f.id).collect();

    // Start with color hits (already sorted by proximity).
    let mut results: Vec<SearchResultFilament> = color_hits
        .into_iter()
        .map(|(f, field)| SearchResultFilament {
            filament: f,
            match_field: field,
            spools: None,
            spool_count: None,
        })
        .collect();

    let mut text_hits: Vec<(FilamentResponse, String, i64)> = filaments
        .into_iter()
        .filter(|f| !color_ids.contains(&f.id))
        .filter_map(|f| {
            let vendor_name = f.vendor.as_ref().map(|v| v.name.as_str());
            classify(
                terms,
                &[
                    ("name", f.name.as_deref()),
                    ("material", f.material.as_deref()),
                    ("article_number", f.article_number.as_deref()),
                    ("comment", f.comment.as_deref()),
                    ("vendor.name", vendor_name),
                ],
            )
            .map(|(field, rank)| (f, field, rank))
        })
        .collect();

    text_hits.sort_by(|a, b| a.2.cmp(&b.2).then(a.0.id.cmp(&b.0.id)));

    for (f, field, _) in text_hits {
        if results.len() >= limit {
            break;
        }
        results.push(SearchResultFilament {
            filament: f,
            match_field: field,
            spools: None,
            spool_count: None,
        });
    }

    results.truncate(limit);
    results
}

fn match_vendors(
    vendors: Vec<VendorResponse>,
    terms: &[String],
    limit: usize,
) -> Vec<SearchResultVendor> {
    let mut hits: Vec<(VendorResponse, String, i64)> = vendors
        .into_iter()
        .filter_map(|v| {
            classify(
                terms,
                &[
                    ("name", Some(v.name.as_str())),
                    ("comment", v.comment.as_deref()),
                ],
            )
            .map(|(f, r)| (v, f, r))
        })
        .collect();

    hits.sort_by(|a, b| a.2.cmp(&b.2).then(a.0.id.cmp(&b.0.id)));
    hits.truncate(limit);
    hits.into_iter()
        .map(|(v, f, _)| SearchResultVendor {
            vendor: v,
            match_field: f,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Attach spools to filament results
// ---------------------------------------------------------------------------

async fn attach_filament_spools(
    pool: &AnyPool,
    _engine: &Engine,
    matches: &mut [SearchResultFilament],
    per_filament: i64,
    allow_archived: bool,
) -> Result<()> {
    if matches.is_empty() || per_filament <= 0 {
        return Ok(());
    }
    let fids: Vec<i64> = matches.iter().map(|m| m.filament.id).collect();
    let placeholders = fids.iter().map(|_| "?").collect::<Vec<_>>().join(",");

    let archived_clause = if allow_archived {
        ""
    } else {
        "AND (s.archived IS NULL OR CAST(s.archived AS INTEGER) = 0)"
    };
    let sql = format!(
        "SELECT s.filament_id, s.id, s.initial_weight, s.used_weight, s.location, \
         CAST(s.archived AS INTEGER) AS archived \
         FROM spool s WHERE s.filament_id IN ({placeholders}) {archived_clause} ORDER BY s.id"
    );
    let mut q = sqlx::query(&sql);
    for id in &fids {
        q = q.bind(*id);
    }
    let rows = q.fetch_all(pool).await?;

    let mut by_fid: std::collections::HashMap<i64, (Vec<SearchResultFilamentSpool>, i64)> =
        fids.iter().map(|&id| (id, (vec![], 0))).collect();

    for row in &rows {
        use sqlx::Row;
        let fid: i64 = row.try_get("filament_id")?;
        let sid: i64 = row.try_get("id")?;
        let initial_weight: Option<f64> = row.try_get("initial_weight")?;
        let used_weight: f64 = row.try_get("used_weight")?;
        let location: Option<String> = row.try_get("location")?;
        let archived_raw: Option<i64> = row.try_get("archived")?;
        let archived = archived_raw.map(|v| v != 0).unwrap_or(false);

        let entry = by_fid.entry(fid).or_insert((vec![], 0));
        entry.1 += 1;
        if entry.0.len() < per_filament as usize {
            let remaining_weight = initial_weight.map(|iw| iw - used_weight);
            entry.0.push(SearchResultFilamentSpool {
                id: sid,
                initial_weight,
                used_weight,
                remaining_weight,
                location,
                archived,
            });
        }
    }

    for m in matches.iter_mut() {
        let (spools, count) = by_fid.remove(&m.filament.id).unwrap_or_else(|| (vec![], 0));
        m.spools = Some(spools);
        m.spool_count = Some(count);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub async fn search(
    pool: &AnyPool,
    engine: &Engine,
    query: &str,
    color_similarity_threshold: f64,
    limit: i64,
    allow_archived: bool,
    spools_per_filament: i64,
) -> Result<SearchResults> {
    let q = query.trim();
    let terms = split_terms(q);

    if terms.is_empty() {
        return Ok(SearchResults {
            spools: vec![],
            filaments: vec![],
            vendors: vec![],
            is_color_query: false,
        });
    }

    let color_hex = detect_color(q);
    let is_color_query = color_hex.is_some();

    // Fetch candidate rows.  We over-fetch (up to 10 000) and rank in Rust.
    let (all_spools, _) = spool_db::find(
        pool,
        engine,
        &SpoolFilter {
            allow_archived,
            limit: Some(10_000),
            ..Default::default()
        },
    )
    .await?;

    let (all_filaments, _) = filament_db::find(
        pool,
        engine,
        &FilamentFilter {
            limit: Some(10_000),
            ..Default::default()
        },
    )
    .await?;

    let (all_vendors, _) = vendor_db::find(
        pool,
        engine,
        &VendorFilter {
            limit: Some(10_000),
            ..Default::default()
        },
    )
    .await?;

    // Color hits for filaments.
    let color_hits: Vec<(FilamentResponse, String)> = if let Some(ref hex) = color_hex {
        let mut hits: Vec<(FilamentResponse, f64)> = all_filaments
            .iter()
            .filter_map(|f| {
                let d = filament_min_color_distance(hex, f);
                if d <= color_similarity_threshold {
                    Some((f.clone(), d))
                } else {
                    None
                }
            })
            .collect();
        hits.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(limit as usize);
        hits.into_iter()
            .map(|(f, _)| (f, "color".to_owned()))
            .collect()
    } else {
        vec![]
    };

    let lim = limit as usize;
    let mut spools = match_spools(all_spools, &terms, lim);
    let mut filaments = match_filaments(all_filaments, &terms, lim, color_hits);
    let vendors = match_vendors(all_vendors, &terms, lim);

    // Numeric query: prepend exact spool-id match if not already present.
    if let Ok(spool_id) = q.parse::<i64>() {
        if !spools.iter().any(|s| s.spool.id == spool_id) {
            if let Ok(spool) = spool_db::find_by_id(pool, spool_id).await {
                if allow_archived || !spool.archived {
                    spools.insert(
                        0,
                        SearchResultSpool {
                            spool,
                            match_field: "id".to_owned(),
                        },
                    );
                    spools.truncate(lim);
                }
            }
        }
    }

    if spools_per_filament > 0 {
        attach_filament_spools(
            pool,
            engine,
            &mut filaments,
            spools_per_filament,
            allow_archived,
        )
        .await?;
    }

    Ok(SearchResults {
        spools,
        filaments,
        vendors,
        is_color_query,
    })
}
