//! Inventory rules (task 4.3): stock status per tracked filament, what to buy,
//! and how much filament queued jobs have already claimed.
//!
//! Pure functions; the server gathers spools, queue items and loaded slots and
//! feeds them in here.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A spool with this little left (after reservations) no longer counts as stock.
pub const EMPTY_GRAMS: f64 = 50.0;

/// Stock state of a tracked filament.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum StockStatus {
    /// At or above its buy-below level.
    Ok,
    /// Below buy-below: on the shopping list.
    Buy,
    /// No usable spool left.
    Out,
}

/// Status and how many spools to buy, from usable spool count, target and buy-below.
///
/// - `Out` when nothing usable is left, `Buy` when below buy-below, else `Ok`.
/// - Buy count = target − stock, only when below buy-below (else 0).
pub fn stock_status(stock: u32, target: u32, buy_below: u32) -> (StockStatus, u32) {
    let buy_below = buy_below.min(target);
    let need = stock < buy_below;
    let buy = if need {
        target.saturating_sub(stock)
    } else {
        0
    };
    let status = if stock == 0 {
        StockStatus::Out
    } else if need {
        StockStatus::Buy
    } else {
        StockStatus::Ok
    };
    (status, buy)
}

/// Spools that still count as stock: grams left after reservations > [`EMPTY_GRAMS`].
pub fn usable(free_grams: impl IntoIterator<Item = f64>) -> u32 {
    free_grams.into_iter().filter(|g| *g > EMPTY_GRAMS).count() as u32
}

// ---------------------------------------------------------------------------
// Reserved weight
// ---------------------------------------------------------------------------

/// One tool's use in a queued job.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDemand {
    pub tool: u32,
    pub grams: f64,
    /// The file was sliced for this Spoolman spool (a `… (#ID) - Spoolman` preset).
    pub spool_id: Option<i64>,
}

/// A queue item that is waiting, sending or printing.
#[derive(Debug, Clone, PartialEq)]
pub struct JobDemand {
    pub queue_item_id: i64,
    pub file_name: String,
    /// Lane: `None` = fleet queue (no printer picked yet).
    pub printer_id: Option<i64>,
    pub tools: Vec<ToolDemand>,
    /// Share of the job still to print: 1.0 when queued, `1 − progress` while printing.
    pub left: f64,
}

/// Which spool sits in which tool of which printer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotSpool {
    pub printer_id: i64,
    pub tool: u32,
    pub spool_id: i64,
}

/// Grams one job holds on one spool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Reservation {
    #[ts(type = "number")]
    pub spool_id: i64,
    #[ts(type = "number")]
    pub queue_item_id: i64,
    pub file_name: String,
    pub grams: f64,
}

/// Reservations from queued / printing jobs.
///
/// A job commits a spool when the file names one (sliced for that spool) or
/// when it sits in a printer's lane and that printer has a spool in the tool.
/// Fleet-queue jobs with no named spool hold nothing yet. A spool never has
/// more reserved than it holds (`remaining`): queue order decides who gets it.
pub fn reservations(
    jobs: &[JobDemand],
    slots: &[SlotSpool],
    remaining: &HashMap<i64, f64>,
) -> Vec<Reservation> {
    let mut left: HashMap<i64, f64> = remaining.clone();
    let mut out = Vec::new();
    for job in jobs {
        let share = job.left.clamp(0.0, 1.0);
        for t in &job.tools {
            let grams = t.grams * share;
            if grams.is_nan() || grams <= 0.0 {
                continue;
            }
            let spool = t.spool_id.or_else(|| {
                let pid = job.printer_id?;
                slots
                    .iter()
                    .find(|s| s.printer_id == pid && s.tool == t.tool)
                    .map(|s| s.spool_id)
            });
            let Some(spool_id) = spool else { continue };
            let Some(avail) = left.get_mut(&spool_id) else {
                continue;
            };
            let g = grams.min(*avail);
            if g <= 0.0 {
                continue;
            }
            *avail -= g;
            match out.iter_mut().find(|r: &&mut Reservation| {
                r.spool_id == spool_id && r.queue_item_id == job.queue_item_id
            }) {
                Some(r) => r.grams += g,
                None => out.push(Reservation {
                    spool_id,
                    queue_item_id: job.queue_item_id,
                    file_name: job.file_name.clone(),
                    grams: g,
                }),
            }
        }
    }
    out
}

/// Total reserved grams per spool.
pub fn reserved_by_spool(rs: &[Reservation]) -> HashMap<i64, f64> {
    let mut m = HashMap::new();
    for r in rs {
        *m.entry(r.spool_id).or_insert(0.0) += r.grams;
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_thresholds() {
        // target 3, buy below 2
        assert_eq!(stock_status(0, 3, 2), (StockStatus::Out, 3));
        assert_eq!(stock_status(1, 3, 2), (StockStatus::Buy, 2));
        assert_eq!(stock_status(2, 3, 2), (StockStatus::Ok, 0));
        assert_eq!(stock_status(3, 3, 2), (StockStatus::Ok, 0));
        assert_eq!(stock_status(5, 3, 2), (StockStatus::Ok, 0));
        // buy-below equal to target: any shortfall goes on the list
        assert_eq!(stock_status(2, 3, 3), (StockStatus::Buy, 1));
        // buy-below 0: never on the list, but empty still shows OUT
        assert_eq!(stock_status(0, 2, 0), (StockStatus::Out, 0));
        // buy-below above target is clamped to the target
        assert_eq!(stock_status(2, 2, 5), (StockStatus::Ok, 0));
        assert_eq!(stock_status(1, 2, 5), (StockStatus::Buy, 1));
        // target 0: nothing wanted
        assert_eq!(stock_status(0, 0, 0), (StockStatus::Out, 0));
    }

    #[test]
    fn buy_count_is_target_minus_stock() {
        for target in 0..6u32 {
            for below in 0..=target {
                for stock in 0..8u32 {
                    let (_, buy) = stock_status(stock, target, below);
                    let want = if stock < below { target - stock } else { 0 };
                    assert_eq!(buy, want, "stock {stock} target {target} below {below}");
                    if buy > 0 {
                        assert!(stock + buy == target);
                    }
                }
            }
        }
    }

    #[test]
    fn usable_skips_nearly_empty_spools() {
        assert_eq!(usable([1000.0, 51.0, 50.0, 0.0, -3.0]), 2);
    }

    fn job(
        id: i64,
        printer: Option<i64>,
        tools: &[(u32, f64, Option<i64>)],
        left: f64,
    ) -> JobDemand {
        JobDemand {
            queue_item_id: id,
            file_name: format!("job{id}.gcode"),
            printer_id: printer,
            tools: tools
                .iter()
                .map(|&(tool, grams, spool_id)| ToolDemand {
                    tool,
                    grams,
                    spool_id,
                })
                .collect(),
            left,
        }
    }

    #[test]
    fn queued_jobs_reserve_the_loaded_spool() {
        let slots = [
            SlotSpool {
                printer_id: 1,
                tool: 0,
                spool_id: 10,
            },
            SlotSpool {
                printer_id: 2,
                tool: 0,
                spool_id: 20,
            },
            SlotSpool {
                printer_id: 2,
                tool: 1,
                spool_id: 21,
            },
        ];
        let remaining = HashMap::from([(10, 800.0), (20, 500.0), (21, 300.0), (30, 1000.0)]);
        let jobs = [
            // printing on printer 1, 40 % done → 60 % of 100 g
            job(1, Some(1), &[(0, 100.0, None)], 0.6),
            // waiting on printer 1
            job(2, Some(1), &[(0, 50.0, None)], 1.0),
            // two-tool job on printer 2
            job(3, Some(2), &[(0, 30.0, None), (1, 20.0, None)], 1.0),
            // fleet job, no spool named: holds nothing
            job(4, None, &[(0, 999.0, None)], 1.0),
            // fleet job sliced for spool 30
            job(5, None, &[(0, 120.0, Some(30))], 1.0),
            // printer 3 has nothing loaded: holds nothing
            job(6, Some(3), &[(0, 77.0, None)], 1.0),
        ];
        let rs = reservations(&jobs, &slots, &remaining);
        let by = reserved_by_spool(&rs);
        assert!((by[&10] - 110.0).abs() < 1e-9, "{by:?}");
        assert_eq!(by[&20], 30.0);
        assert_eq!(by[&21], 20.0);
        assert_eq!(by[&30], 120.0);
        assert_eq!(by.len(), 4);
        assert_eq!(rs.iter().filter(|r| r.spool_id == 10).count(), 2);
    }

    #[test]
    fn a_spool_is_never_reserved_past_what_it_holds() {
        let slots = [SlotSpool {
            printer_id: 1,
            tool: 0,
            spool_id: 10,
        }];
        let remaining = HashMap::from([(10, 120.0)]);
        let jobs = [
            job(1, Some(1), &[(0, 100.0, None)], 1.0),
            job(2, Some(1), &[(0, 100.0, None)], 1.0),
            job(3, Some(1), &[(0, 100.0, None)], 1.0),
        ];
        let rs = reservations(&jobs, &slots, &remaining);
        assert_eq!(rs.len(), 2);
        assert_eq!(rs[0].grams, 100.0);
        assert_eq!(rs[1].grams, 20.0);
        assert_eq!(reserved_by_spool(&rs)[&10], 120.0);
    }

    #[test]
    fn unknown_or_archived_spools_hold_nothing() {
        // spool 99 is named by the file but is not in stock (archived / deleted)
        let rs = reservations(
            &[job(1, None, &[(0, 50.0, Some(99))], 1.0)],
            &[],
            &HashMap::new(),
        );
        assert!(rs.is_empty());
    }
}
