//! Materials + modifiers (W8, task 4.2).
//!
//! - The base materials the filament form offers, with default temperatures and
//!   density (the form prefills them when the material changes).
//! - The Marathon modifier set, grouped like the form shows it.
//! - [`auto_abrasive`]: the rule that ticks the abrasive box (CF, GF, AF, Metal,
//!   Glow). The flag is then stored on the filament and the user can untick it;
//!   the guard and compat read the stored flag.
//! - [`normalize_filament_type`] / [`is_abrasive_filament`]: Rust ports of the
//!   old app's `materialUtils.js`, for material strings that come with no flag
//!   (slicer metadata, legacy filaments created before the flag existed).

use serde::Serialize;
use ts_rs::TS;

// ---------------------------------------------------------------------------
// Catalogue
// ---------------------------------------------------------------------------

/// One base material with the values the form prefills.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct MaterialInfo {
    pub name: String,
    #[ts(type = "number")]
    pub nozzle_temp: i64,
    #[ts(type = "number")]
    pub bed_temp: i64,
    /// g/cm³.
    pub density: f64,
    /// Short handling note shown under the print settings ("Needs an enclosure.").
    pub hint: Option<String>,
}

/// One modifier chip.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ModifierInfo {
    pub name: String,
    /// Picking it ticks "Abrasive".
    pub abrasive: bool,
    /// Why it is abrasive ("carbon fibre"), for the "auto: …" note.
    pub why: Option<String>,
}

/// A labelled group of modifier chips ("Finish", "Filled", …).
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ModifierGroup {
    pub label: String,
    pub items: Vec<ModifierInfo>,
}

/// Everything the filament form needs to offer materials and modifiers.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct MaterialCatalogue {
    pub materials: Vec<MaterialInfo>,
    pub modifier_groups: Vec<ModifierGroup>,
}

/// name, nozzle °C, bed °C, density, hint.
const MATERIALS: &[(&str, i64, i64, f64, Option<&str>)] = &[
    ("PLA", 215, 60, 1.24, None),
    ("PLA+", 220, 60, 1.24, None),
    ("PETG", 240, 80, 1.27, None),
    ("ABS", 250, 100, 1.04, Some("Needs an enclosure.")),
    ("ASA", 255, 100, 1.07, Some("Needs an enclosure.")),
    (
        "TPU",
        225,
        50,
        1.21,
        Some("Flexible: slow speeds; direct drive only."),
    ),
    ("PA", 270, 90, 1.08, Some("Nylon: dry before use.")),
    (
        "PC",
        270,
        110,
        1.20,
        Some("Needs an enclosure and high bed temp."),
    ),
    (
        "HIPS",
        235,
        100,
        1.04,
        Some("Support for ABS; needs an enclosure."),
    ),
    (
        "PVA",
        200,
        60,
        1.23,
        Some("Water-soluble support: keep dry."),
    ),
];

/// The Marathon modifier set, in form order.
const MODIFIER_GROUPS: &[(&str, &[&str])] = &[
    (
        "Finish",
        &[
            "Silk",
            "Silk BiColor",
            "Silk TriColor",
            "Silk Rainbow",
            "Matte",
            "Glitter",
            "Galaxy",
            "Glow",
            "Metal",
            "Translucent",
        ],
    ),
    ("Filled", &["CF", "GF", "AF"]),
    ("Performance", &["HF", "HS", "HT", "Tough", "Pro"]),
    ("Shore", &["95A", "85A", "70A", "60A"]),
];

/// Modifiers that make a filament abrasive, and why.
const ABRASIVE_MODIFIERS: &[(&str, &str)] = &[
    ("CF", "carbon fibre"),
    ("GF", "glass fibre"),
    ("AF", "aramid fibre"),
    ("Metal", "metal-filled"),
    ("Glow", "glow pigment"),
];

/// The base material names the form offers.
pub fn base_materials() -> impl Iterator<Item = &'static str> {
    MATERIALS.iter().map(|m| m.0)
}

/// Defaults for a base material (case-insensitive), if it is one we know.
pub fn material_info(name: &str) -> Option<MaterialInfo> {
    MATERIALS
        .iter()
        .find(|m| m.0.eq_ignore_ascii_case(name.trim()))
        .map(|&(name, n, b, d, h)| MaterialInfo {
            name: name.into(),
            nozzle_temp: n,
            bed_temp: b,
            density: d,
            hint: h.map(Into::into),
        })
}

/// The whole catalogue, for `GET /api/filament/catalogue`.
pub fn catalogue() -> MaterialCatalogue {
    MaterialCatalogue {
        materials: base_materials().filter_map(material_info).collect(),
        modifier_groups: MODIFIER_GROUPS
            .iter()
            .map(|(label, items)| ModifierGroup {
                label: (*label).into(),
                items: items
                    .iter()
                    .map(|m| ModifierInfo {
                        name: (*m).into(),
                        abrasive: abrasive_reason(m).is_some(),
                        why: abrasive_reason(m).map(Into::into),
                    })
                    .collect(),
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Modifier lists (stored space-separated in `material_modifier`)
// ---------------------------------------------------------------------------

fn known_modifiers() -> impl Iterator<Item = &'static str> {
    MODIFIER_GROUPS
        .iter()
        .flat_map(|(_, items)| items.iter().copied())
}

/// Split a `material_modifier` value into modifiers. Known multi-word modifiers
/// ("Silk BiColor") stay whole; separators are spaces, commas, `-` and `+`.
/// Case follows the catalogue for known modifiers; unknown words are kept as typed.
pub fn split_modifiers(s: &str) -> Vec<String> {
    let words: Vec<&str> = s
        .split(|c: char| c.is_whitespace() || c == ',' || c == '-' || c == '+')
        .filter(|w| !w.is_empty())
        .collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < words.len() {
        // Longest known modifier starting at word i.
        let mut best: Option<(&str, usize)> = None;
        for known in known_modifiers() {
            let parts: Vec<&str> = known.split(' ').collect();
            let n = parts.len();
            if i + n <= words.len()
                && parts
                    .iter()
                    .zip(&words[i..i + n])
                    .all(|(a, b)| a.eq_ignore_ascii_case(b))
                && best.is_none_or(|(_, bn)| n > bn)
            {
                best = Some((known, n));
            }
        }
        let (word, n) = best.unwrap_or((words[i], 1));
        if !out.iter().any(|o| o.eq_ignore_ascii_case(word)) {
            out.push(word.to_owned());
        }
        i += n;
    }
    out
}

/// Join modifiers for storage: "CF HF".
pub fn join_modifiers(mods: &[String]) -> String {
    mods.iter()
        .map(|m| m.trim())
        .filter(|m| !m.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Why a modifier is abrasive ("carbon fibre"), or `None`.
/// `CF`, `cf`, `GF30` (fill percentage) all count.
pub fn abrasive_reason(modifier: &str) -> Option<&'static str> {
    let m = modifier.trim();
    ABRASIVE_MODIFIERS.iter().find_map(|(k, why)| {
        let hit = if k.len() == 2 {
            // CF / GF / AF, optionally followed by a fill percentage.
            m.get(..2).is_some_and(|p| p.eq_ignore_ascii_case(k))
                && m[2..].chars().all(|c| c.is_ascii_digit())
        } else {
            m.eq_ignore_ascii_case(k)
        };
        hit.then_some(*why)
    })
}

/// The abrasive box auto-ticks when any modifier is abrasive (CF, GF, AF, Metal, Glow).
pub fn auto_abrasive(mods: &[String]) -> bool {
    mods.iter().any(|m| abrasive_reason(m).is_some())
}

// ---------------------------------------------------------------------------
// materialUtils.js port
// ---------------------------------------------------------------------------

/// Canonical base types; longer names first where one prefixes another.
const BASE_TYPES: &[&str] = &[
    "PETG", "PLA", "ABS", "ASA", "HIPS", "PVA", "TPU", "TPE", "PC", "PA", "FLEX", "NYLON",
];

/// Variant suffixes that do not change the base polymer (`VARIANT_RE`).
const VARIANTS: &[&str] = &[
    "HF",
    "HS",
    "LW",
    "Silk Rainbow",
    "Silk TriColor",
    "Silk BiColor",
    "Silk",
    "Matte",
    "Tough",
    "Pro",
    "Max",
    "Ultra",
    "Basic",
    "Speed",
    "Rapid",
    "Galaxy",
    "Glitter",
    "Glow",
    "Metal",
    "BiColor",
    "TriColor",
    "Rainbow",
    "Clear",
    "Transparent",
    "Translucent",
    "Eco",
    "Bio",
    "ST",
    "Plus",
];

/// True for CF / GF / AF with an optional fill number (`ABRASIVE_RE`).
fn is_filler_token(t: &str) -> bool {
    t.len() >= 2
        && ["CF", "GF", "AF"]
            .iter()
            .any(|p| t[..2].eq_ignore_ascii_case(p))
        && t[2..].chars().all(|c| c.is_ascii_digit())
}

/// Normalised base polymer of a material string.
/// "PETG HF" → "PETG", "PLA Silk" → "PLA", "PA12-CF" → "PA", "ASA+" → "ASA".
pub fn normalize_filament_type(material: &str) -> String {
    let m = material.trim();
    if m.is_empty() {
        return String::new();
    }
    // Tokens split on space / '-' ; '+' becomes its own variant token.
    let spaced = m.replace('+', " + ");
    let mut tokens: Vec<&str> = spaced
        .split(|c: char| c.is_whitespace() || c == '-')
        .filter(|t| !t.is_empty())
        .collect();
    // Strip a trailing filler suffix first.
    if tokens.len() > 1 && tokens.last().is_some_and(|t| is_filler_token(t)) {
        tokens.pop();
    }
    // Strip variant suffixes (multi-word variants first).
    let mut kept: Vec<&str> = Vec::new();
    let mut i = 0;
    'outer: while i < tokens.len() {
        if tokens[i] == "+" {
            i += 1;
            continue;
        }
        if i > 0 {
            for v in VARIANTS {
                let parts: Vec<&str> = v.split(' ').collect();
                let n = parts.len();
                if i + n <= tokens.len()
                    && parts
                        .iter()
                        .zip(&tokens[i..i + n])
                        .all(|(a, b)| a.eq_ignore_ascii_case(b))
                {
                    i += n;
                    continue 'outer;
                }
            }
        }
        kept.push(tokens[i]);
        i += 1;
    }
    let upper = kept.join(" ").to_ascii_uppercase();
    for base in BASE_TYPES {
        if upper == *base
            || upper.starts_with(&format!("{base}-"))
            || upper.starts_with(&format!("{base} "))
        {
            return (*base).into();
        }
        if upper.trim_end_matches(|c: char| c.is_ascii_digit()) == *base {
            return (*base).into();
        }
    }
    if upper.is_empty() {
        m.to_ascii_uppercase()
    } else {
        upper
    }
}

/// True when the material or modifier names an abrasive filament.
/// Ported from `isAbrasiveFilament` (CF / GF / AF with an optional number),
/// extended with Metal and Glow to match the form's auto-tick rule.
/// Use only where no stored `abrasive` flag exists.
pub fn is_abrasive_filament(material: Option<&str>, modifier: Option<&str>) -> bool {
    [material, modifier].into_iter().flatten().any(|s| {
        s.split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|t| !t.is_empty())
            .any(|t| abrasive_reason(t).is_some())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── normalizeFilamentType (ported tests) ────────────────────────────────

    #[test]
    fn returns_plain_base_types_unchanged() {
        for m in ["PLA", "PETG", "ABS", "ASA", "TPU", "PA"] {
            assert_eq!(normalize_filament_type(m), m);
        }
    }

    #[test]
    fn strips_variant_suffixes() {
        assert_eq!(normalize_filament_type("PETG HF"), "PETG");
        assert_eq!(normalize_filament_type("PLA Silk"), "PLA");
        assert_eq!(normalize_filament_type("PLA Matte"), "PLA");
        assert_eq!(normalize_filament_type("PLA+"), "PLA");
        assert_eq!(normalize_filament_type("ABS+"), "ABS");
        assert_eq!(normalize_filament_type("PETG-HS"), "PETG");
        assert_eq!(normalize_filament_type("PLA Pro"), "PLA");
        assert_eq!(normalize_filament_type("PLA Silk TriColor"), "PLA");
    }

    #[test]
    fn strips_abrasive_suffixes_and_returns_base_type() {
        assert_eq!(normalize_filament_type("ASA CF"), "ASA");
        assert_eq!(normalize_filament_type("PA-CF"), "PA");
        assert_eq!(normalize_filament_type("PA12-CF"), "PA");
        assert_eq!(normalize_filament_type("PETG-GF"), "PETG");
        assert_eq!(normalize_filament_type("ABS-CF"), "ABS");
    }

    #[test]
    fn handles_pa_with_number_variants() {
        assert_eq!(normalize_filament_type("PA12"), "PA");
        assert_eq!(normalize_filament_type("PA6"), "PA");
    }

    #[test]
    fn handles_empty_gracefully() {
        assert_eq!(normalize_filament_type(""), "");
        assert_eq!(normalize_filament_type("   "), "");
    }

    // ── isAbrasiveFilament (ported tests) ───────────────────────────────────

    #[test]
    fn detects_cf_materials() {
        for m in ["ASA CF", "PA-CF", "PA12-CF", "ABS-CF"] {
            assert!(is_abrasive_filament(Some(m), None), "{m}");
        }
    }

    #[test]
    fn detects_gf_materials() {
        assert!(is_abrasive_filament(Some("PETG-GF"), None));
        assert!(is_abrasive_filament(Some("PA GF"), None));
        assert!(is_abrasive_filament(Some("PA6-GF30"), None));
    }

    #[test]
    fn returns_false_for_non_abrasive() {
        for m in ["PLA", "PETG HF", "PLA Silk", "ASA", ""] {
            assert!(!is_abrasive_filament(Some(m), None), "{m}");
        }
        assert!(!is_abrasive_filament(None, None));
        assert!(!is_abrasive_filament(Some("CFRP-ish"), None));
    }

    #[test]
    fn modifier_field_counts_too() {
        assert!(is_abrasive_filament(Some("PETG"), Some("CF")));
        assert!(is_abrasive_filament(Some("PLA"), Some("Glow")));
        assert!(is_abrasive_filament(Some("PLA"), Some("Silk Metal")));
        assert!(!is_abrasive_filament(Some("PLA"), Some("Silk BiColor")));
    }

    // ── Modifier → abrasive matrix ──────────────────────────────────────────

    #[test]
    fn modifier_abrasive_matrix() {
        let cat = catalogue();
        let abrasive: Vec<&str> = cat
            .modifier_groups
            .iter()
            .flat_map(|g| g.items.iter())
            .filter(|m| m.abrasive)
            .map(|m| m.name.as_str())
            .collect();
        assert_eq!(abrasive, ["Glow", "Metal", "CF", "GF", "AF"]);
        for g in &cat.modifier_groups {
            for m in &g.items {
                assert_eq!(
                    auto_abrasive(std::slice::from_ref(&m.name)),
                    m.abrasive,
                    "auto_abrasive({}) disagrees with the catalogue",
                    m.name
                );
                assert_eq!(m.why.is_some(), m.abrasive);
            }
        }
        // Combinations: one abrasive modifier is enough.
        let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert!(auto_abrasive(&v(&["HF", "CF"])));
        assert!(auto_abrasive(&v(&["Silk", "Glow"])));
        assert!(!auto_abrasive(&v(&["Silk", "Matte", "HF", "95A"])));
        assert!(!auto_abrasive(&[]));
        assert!(auto_abrasive(&v(&["gf30"])));
    }

    #[test]
    fn split_and_join_modifiers() {
        assert_eq!(split_modifiers("CF HF"), ["CF", "HF"]);
        assert_eq!(split_modifiers("silk bicolor"), ["Silk BiColor"]);
        assert_eq!(
            split_modifiers("Silk Rainbow, Matte"),
            ["Silk Rainbow", "Matte"]
        );
        assert_eq!(split_modifiers("CF-HS"), ["CF", "HS"]);
        assert_eq!(split_modifiers("Sparkly cf"), ["Sparkly", "CF"]);
        assert_eq!(split_modifiers("CF cf"), ["CF"]);
        assert!(split_modifiers("  ").is_empty());
        let mods = split_modifiers("Silk TriColor HS");
        assert_eq!(join_modifiers(&mods), "Silk TriColor HS");
        assert_eq!(split_modifiers(&join_modifiers(&mods)), mods);
    }

    #[test]
    fn catalogue_has_the_brief_materials_with_defaults() {
        let names: Vec<&str> = base_materials().collect();
        assert_eq!(
            names,
            ["PLA", "PLA+", "PETG", "ABS", "ASA", "TPU", "PA", "PC", "HIPS", "PVA"]
        );
        let petg = material_info("petg").map(|m| (m.nozzle_temp, m.bed_temp));
        assert_eq!(petg, Some((240, 80)));
        assert!(material_info("Unobtainium").is_none());
    }
}
