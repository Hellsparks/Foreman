//! Filament tracking mode (doc 07 §A).
//!
//! Settings → Filament: **Full** (built-in Spoolman), **External** (one-time
//! import, then as Full) or **Off** (fleet dashboard only). Everything that
//! differs between the modes is decided here, in [`FilamentMode::policy`]:
//! which compatibility verdicts exist, whether spools take part at all, and the
//! default for serving the Spoolman API. Callers ask the policy; they never
//! compare the mode themselves.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::compat::Verdict;

/// How Foreman tracks filament.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum FilamentMode {
    /// Built-in Spoolman (the behaviour before doc 07).
    #[default]
    Full,
    /// Records imported once from another Spoolman, then tracked as in Full.
    External,
    /// No spools: no guard, reservations, deduction or spool verdicts.
    Off,
}

/// Verdicts that need spools. They exist only while spools are tracked.
const SPOOL_VERDICTS: &[Verdict] = &[Verdict::Material, Verdict::NoSpool];

/// Every verdict, in display order.
const ALL_VERDICTS: &[Verdict] = &[
    Verdict::Ready,
    Verdict::Busy,
    Verdict::Plate,
    Verdict::Material,
    Verdict::NoSpool,
    Verdict::TooBig,
    Verdict::Nozzle,
    Verdict::Offline,
    Verdict::Unsupported,
    Verdict::CloudOnly,
];

/// What a mode allows, sent to the UI as-is so it never re-derives it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FilamentPolicy {
    pub mode: FilamentMode,
    /// Spools take part: inventory, guard, reservations, deduction, spool
    /// pickers, the Filament section and the printer Spools tab.
    pub spools: bool,
    /// Compatibility verdicts this mode can produce, in display order.
    pub verdicts: Vec<Verdict>,
    /// Default for serving the Spoolman API (port 7912) when not set explicitly.
    pub serve_api_default: bool,
}

impl FilamentMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::External => "external",
            Self::Off => "off",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "full" => Some(Self::Full),
            "external" => Some(Self::External),
            "off" => Some(Self::Off),
            _ => None,
        }
    }

    /// Spools take part in anything (the one switch every spool path reads).
    pub fn tracks_spools(self) -> bool {
        self != Self::Off
    }

    /// Whether this mode can produce `v`.
    pub fn allows(self, v: &Verdict) -> bool {
        self.tracks_spools() || !SPOOL_VERDICTS.contains(v)
    }

    pub fn policy(self) -> FilamentPolicy {
        FilamentPolicy {
            mode: self,
            spools: self.tracks_spools(),
            verdicts: ALL_VERDICTS
                .iter()
                .filter(|v| self.allows(v))
                .cloned()
                .collect(),
            serve_api_default: self.tracks_spools(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_drops_exactly_the_spool_verdicts() {
        let off = FilamentMode::Off.policy().verdicts;
        assert!(!off.contains(&Verdict::Material));
        assert!(!off.contains(&Verdict::NoSpool));
        assert_eq!(off.len(), ALL_VERDICTS.len() - SPOOL_VERDICTS.len());
        assert_eq!(FilamentMode::Full.policy().verdicts, ALL_VERDICTS.to_vec());
        assert_eq!(
            FilamentMode::External.policy().verdicts,
            ALL_VERDICTS.to_vec()
        );
    }

    #[test]
    fn mode_round_trips() {
        for m in [
            FilamentMode::Full,
            FilamentMode::External,
            FilamentMode::Off,
        ] {
            assert_eq!(FilamentMode::parse(m.as_str()), Some(m));
        }
        assert_eq!(FilamentMode::parse("nope"), None);
    }
}
