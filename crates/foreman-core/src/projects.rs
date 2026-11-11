//! Templates and projects (task 5.1, W9): types and the counting rules.
//!
//! Copies are counted as **prints**, not plates: a plate with 3 copies in a
//! 2-set project is 6 prints. Grams, reservations, progress and the fleet queue
//! all count prints.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::domain::QueueStatus;

/// Most copies of one plate in one set, and most sets in a project.
pub const MAX_COPIES: u32 = 50;
pub const MAX_SETS: u32 = 50;

// ---------------------------------------------------------------------------
// Template document
// ---------------------------------------------------------------------------

/// Which colour slot feeds one tool of a plate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ToolSlot {
    pub tool: u32,
    /// Slot key ("PRIMARY").
    pub slot: String,
}

/// A colour slot of a template.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TemplateSlot {
    /// User-named key, upper case: PRIMARY, ACCENT, …
    pub key: String,
    /// Preferred colour (`RRGGBB`, no `#`), a suggestion only.
    #[serde(default)]
    pub hex: Option<String>,
    /// Preferred filament in words ("PLA Teal").
    #[serde(default)]
    pub label: Option<String>,
    /// Swap option: any colour may feed this slot. Off = keep to the preferred colour.
    #[serde(default = "yes")]
    pub allow_swap: bool,
}

fn yes() -> bool {
    true
}

/// A plate of a template: a Library file (a copy owned by the template).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TemplatePlate {
    pub name: String,
    #[ts(type = "number")]
    pub file_id: i64,
    /// Copies per set.
    pub copies: u32,
    #[serde(default)]
    pub tools: Vec<ToolSlot>,
}

/// What the template editor saves (and reads back with ids and facts filled in).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TemplateForm {
    pub name: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    /// Library file whose thumbnail stands for the template.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub thumb_file_id: Option<i64>,
    #[serde(default)]
    pub slots: Vec<TemplateSlot>,
    #[serde(default)]
    pub plates: Vec<TemplatePlate>,
}

/// Normalise a slot key: trimmed, upper case, inner spaces kept.
pub fn slot_key(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase()
}

/// Problems that keep a template from being used (empty = ready).
pub fn template_problems(t: &TemplateForm) -> Vec<String> {
    let mut out = Vec::new();
    if t.name.trim().is_empty() {
        out.push("Give the template a name.".into());
    }
    if t.plates.is_empty() {
        out.push("Add at least one plate.".into());
    }
    if t.slots.is_empty() {
        out.push("Add at least one colour slot.".into());
    }
    let mut keys: Vec<String> = t.slots.iter().map(|s| slot_key(&s.key)).collect();
    if keys.iter().any(String::is_empty) {
        out.push("Every slot needs a name.".into());
    }
    keys.sort();
    if keys.windows(2).any(|w| w[0] == w[1]) {
        out.push("Two slots have the same name.".into());
    }
    for p in &t.plates {
        if p.copies == 0 || p.copies > MAX_COPIES {
            out.push(format!("{}: copies per set 1 to {MAX_COPIES}.", p.name));
        }
        if p.tools.is_empty() {
            out.push(format!("{} has no colour slot.", p.name));
        }
        for ts in &p.tools {
            if !t
                .slots
                .iter()
                .any(|s| slot_key(&s.key) == slot_key(&ts.slot))
            {
                out.push(format!(
                    "{} T{} uses unknown slot {}.",
                    p.name, ts.tool, ts.slot
                ));
            }
        }
    }
    for s in &t.slots {
        let k = slot_key(&s.key);
        if !k.is_empty()
            && !t
                .plates
                .iter()
                .any(|p| p.tools.iter().any(|ts| slot_key(&ts.slot) == k))
        {
            out.push(format!("Slot {k} is not used by any plate."));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Counting prints
// ---------------------------------------------------------------------------

/// Where the prints of one project plate stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PlateCounts {
    /// Prints wanted in total.
    pub copies: u32,
    /// Finished.
    pub printed: u32,
    /// Sending or printing now.
    pub printing: u32,
    /// On a queue (waiting, or failed and still on the board).
    pub queued: u32,
    /// Not printed and not on any queue yet.
    pub remaining: u32,
}

/// Count a plate's prints from its stored copies / printed and its live queue items.
pub fn plate_counts(copies: u32, printed: u32, items: &[QueueStatus]) -> PlateCounts {
    let printing = items
        .iter()
        .filter(|s| matches!(s, QueueStatus::Sending | QueueStatus::Printing))
        .count() as u32;
    let queued = items
        .iter()
        .filter(|s| matches!(s, QueueStatus::Waiting | QueueStatus::Failed))
        .count() as u32;
    PlateCounts {
        copies,
        printed,
        printing,
        queued,
        remaining: copies.saturating_sub(printed + printing + queued),
    }
}

/// Fewest copies a running plate may be set to: never below what is printed or
/// printing, and never below 1.
pub fn min_copies(c: &PlateCounts) -> u32 {
    (c.printed + c.printing).max(1)
}

/// Queued (not yet started) prints to take off the queue when copies drop to `new`.
pub fn unqueue_for(c: &PlateCounts, new: u32) -> u32 {
    (c.printed + c.printing + c.queued)
        .saturating_sub(new)
        .min(c.queued)
}

/// Every print of the project is done and nothing is still running or queued.
pub fn all_done(plates: &[PlateCounts]) -> bool {
    !plates.is_empty()
        && plates
            .iter()
            .all(|c| c.printed >= c.copies && c.printing == 0 && c.queued == 0)
}

/// Grams one print of a plate takes from each slot: (slot key, grams).
pub fn grams_per_slot(tools: &[ToolSlot], tool_grams: &[(u32, f64)]) -> Vec<(String, f64)> {
    let mut out: Vec<(String, f64)> = Vec::new();
    for (tool, g) in tool_grams {
        let Some(ts) = tools.iter().find(|t| t.tool == *tool) else {
            continue;
        };
        let k = slot_key(&ts.slot);
        match out.iter_mut().find(|(s, _)| *s == k) {
            Some((_, sum)) => *sum += g,
            None => out.push((k, *g)),
        }
    }
    out
}

/// Distance between two `RRGGBB` colours, 0 (same) to ~441.
pub fn hex_distance(a: &str, b: &str) -> Option<f64> {
    let rgb = |h: &str| -> Option<[f64; 3]> {
        let h = h.trim().trim_start_matches('#');
        if h.len() < 6 {
            return None;
        }
        let p = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok().map(f64::from);
        Some([p(0)?, p(2)?, p(4)?])
    };
    let (x, y) = (rgb(a)?, rgb(b)?);
    Some(((x[0] - y[0]).powi(2) + (x[1] - y[1]).powi(2) + (x[2] - y[2]).powi(2)).sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;
    use QueueStatus::*;

    #[test]
    fn copies_are_prints() {
        let c = plate_counts(6, 2, &[Printing, Waiting, Waiting, Done, Cancelled]);
        assert_eq!(
            c,
            PlateCounts {
                copies: 6,
                printed: 2,
                printing: 1,
                queued: 2,
                remaining: 1
            }
        );
        // Over-queued (copies lowered elsewhere) never goes negative.
        let c = plate_counts(2, 2, &[Waiting]);
        assert_eq!(c.remaining, 0);
        // Failed items stay on the board and count as queued.
        assert_eq!(plate_counts(3, 0, &[Failed]).queued, 1);
    }

    #[test]
    fn copies_never_drop_below_printed_or_printing() {
        let c = plate_counts(6, 2, &[Printing, Waiting, Waiting]);
        assert_eq!(min_copies(&c), 3);
        assert_eq!(min_copies(&plate_counts(4, 0, &[])), 1);
        // Lowering to 4: printed 2 + printing 1 + queued 2 = 5 → take 1 off the queue.
        assert_eq!(unqueue_for(&c, 4), 1);
        assert_eq!(unqueue_for(&c, 3), 2);
        assert_eq!(unqueue_for(&c, 6), 0);
        // Never more than what is queued.
        assert_eq!(unqueue_for(&c, 0), 2);
    }

    #[test]
    fn project_done_only_when_every_print_is() {
        let done = plate_counts(2, 2, &[]);
        assert!(all_done(&[done, plate_counts(1, 1, &[])]));
        assert!(!all_done(&[done, plate_counts(2, 1, &[])]));
        assert!(!all_done(&[done, plate_counts(1, 1, &[Waiting])]));
        assert!(!all_done(&[]));
    }

    #[test]
    fn grams_follow_the_tool_slot_map() {
        let tools = vec![
            ToolSlot {
                tool: 0,
                slot: "primary".into(),
            },
            ToolSlot {
                tool: 1,
                slot: "ACCENT".into(),
            },
            ToolSlot {
                tool: 2,
                slot: "Primary".into(),
            },
        ];
        let g = grams_per_slot(&tools, &[(0, 10.0), (1, 4.0), (2, 1.5), (3, 9.0)]);
        assert_eq!(g, vec![("PRIMARY".into(), 11.5), ("ACCENT".into(), 4.0)]);
    }

    #[test]
    fn template_checklist() {
        let mut t = TemplateForm {
            name: "Chess".into(),
            category: None,
            notes: None,
            thumb_file_id: None,
            slots: vec![
                TemplateSlot {
                    key: "PRIMARY".into(),
                    hex: None,
                    label: None,
                    allow_swap: true,
                },
                TemplateSlot {
                    key: "ACCENT".into(),
                    hex: None,
                    label: None,
                    allow_swap: true,
                },
            ],
            plates: vec![TemplatePlate {
                name: "Pawns".into(),
                file_id: 1,
                copies: 2,
                tools: vec![ToolSlot {
                    tool: 0,
                    slot: "PRIMARY".into(),
                }],
            }],
        };
        assert_eq!(
            template_problems(&t),
            vec!["Slot ACCENT is not used by any plate."]
        );
        t.plates[0].tools.push(ToolSlot {
            tool: 1,
            slot: "accent".into(),
        });
        assert!(template_problems(&t).is_empty());
        t.slots[1].key = "primary ".into();
        assert!(template_problems(&t).contains(&"Two slots have the same name.".to_string()));
    }

    #[test]
    fn hex_distance_works() {
        assert_eq!(hex_distance("#000000", "000000"), Some(0.0));
        assert!(hex_distance("ffffff", "000000").unwrap_or(0.0) > 440.0);
        assert_eq!(hex_distance("zz", "000000"), None);
    }
}
