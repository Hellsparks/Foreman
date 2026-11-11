//! Maintenance tasks (task 5.3, W12): types, templates and the due maths.
//!
//! A task counts one thing — print hours, filament grams or completed jobs — from
//! the printer's job records since it was last done, with an optional calendar
//! interval on top. It is due on whichever of the two reaches 100% first.
//! Counters are never stored: [`crate::repo::maintenance::counters`] sums them from
//! the `job` table, so they cannot drift from the job history.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::AppError;

/// Default "due soon" threshold, percent of the interval.
pub const DEFAULT_DUE_SOON_PCT: f64 = 90.0;
/// Setting key for the due-soon threshold.
pub const DUE_SOON_SETTING: &str = "maintenance.due_soon_pct";
/// One snooze adds this fraction of the interval.
pub const SNOOZE_STEP: f64 = 0.05;
/// Attention kind prefix; the full kind is `maintenance:<task id>`.
pub const ATTENTION_PREFIX: &str = "maintenance:";

/// What a task's interval counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    /// Print hours of jobs that ran to the end or errored.
    Hours,
    /// Filament grams used by those jobs.
    Grams,
    /// Jobs that completed.
    Jobs,
}

impl Basis {
    pub fn as_str(self) -> &'static str {
        match self {
            Basis::Hours => "hours",
            Basis::Grams => "grams",
            Basis::Jobs => "jobs",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "hours" => Some(Basis::Hours),
            "grams" => Some(Basis::Grams),
            "jobs" => Some(Basis::Jobs),
            _ => None,
        }
    }

    /// Unit label used in titles ("212 / 200 print hours").
    pub fn unit(self) -> &'static str {
        match self {
            Basis::Hours => "print hours",
            Basis::Grams => "g filament",
            Basis::Jobs => "jobs",
        }
    }
}

/// Where a task stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum DueState {
    Ok,
    /// At or past the due-soon threshold (Service only, no Attention item).
    Soon,
    /// At or past 100% (raises the Attention item).
    Overdue,
}

/// A recurring service on one printer.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MaintenanceTask {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub printer_id: i64,
    pub name: String,
    pub notes: Option<String>,
    pub basis: Basis,
    pub interval: f64,
    #[ts(type = "number | null")]
    pub every_days: Option<i64>,
    /// Extra allowance as a fraction of the interval (cleared when done).
    pub snooze: f64,
    /// Counter start (last done). None = the printer's whole job history.
    pub since: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Create / edit body.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TaskForm {
    /// Required on create, ignored on edit.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub printer_id: Option<i64>,
    pub name: String,
    #[serde(default)]
    pub notes: Option<String>,
    pub basis: Basis,
    pub interval: f64,
    #[serde(default)]
    #[ts(type = "number | null")]
    pub every_days: Option<i64>,
    /// Create only: start the counter at zero now instead of counting the
    /// printer's whole job history.
    #[serde(default)]
    pub start_now: bool,
}

impl TaskForm {
    pub fn validate(&self) -> Result<(), AppError> {
        if self.name.trim().is_empty() {
            return Err(AppError::InvalidInput("task name is required".into()));
        }
        if !(self.interval.is_finite() && self.interval > 0.0) {
            return Err(AppError::InvalidInput("interval must be above 0".into()));
        }
        if self.basis == Basis::Jobs && self.interval.fract() != 0.0 {
            return Err(AppError::InvalidInput(
                "a job-count interval must be a whole number".into(),
            ));
        }
        if matches!(self.every_days, Some(d) if d <= 0) {
            return Err(AppError::InvalidInput(
                "every N days must be above 0".into(),
            ));
        }
        Ok(())
    }
}

/// What accrued on a printer since a task's counter start.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Counters {
    pub hours: f64,
    pub grams: f64,
    #[ts(type = "number")]
    pub jobs: i64,
}

impl Counters {
    pub fn value(&self, basis: Basis) -> f64 {
        match basis {
            Basis::Hours => self.hours,
            Basis::Grams => self.grams,
            Basis::Jobs => self.jobs as f64,
        }
    }
}

/// Progress towards due.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Progress {
    /// Accrued on the task's basis since the counter start.
    pub value: f64,
    /// Interval including snooze.
    pub limit: f64,
    /// Days since the counter start (or since the task was created).
    pub days: f64,
    /// Calendar interval including snooze, if any.
    pub days_limit: Option<f64>,
    /// The larger of the two ratios (1.0 = due).
    pub ratio: f64,
    /// True when the calendar interval is the one driving `ratio`.
    pub by_days: bool,
    pub state: DueState,
}

fn round_to(v: f64, places: i32) -> f64 {
    let f = 10f64.powi(places);
    (v * f).round() / f
}

/// Where `task` stands given what accrued since its counter start.
pub fn progress(
    task: &MaintenanceTask,
    c: &Counters,
    now: DateTime<Utc>,
    soon_pct: f64,
) -> Progress {
    let value = c.value(task.basis);
    let limit = task.interval * (1.0 + task.snooze);
    let start = task
        .since
        .as_deref()
        .unwrap_or(&task.created_at)
        .parse::<DateTime<Utc>>()
        .unwrap_or(now);
    let days = ((now - start).num_seconds().max(0) as f64) / 86_400.0;
    let days_limit = task.every_days.map(|d| d as f64 * (1.0 + task.snooze));
    let r_count = if limit > 0.0 { value / limit } else { 0.0 };
    let r_days = days_limit.map(|l| days / l).unwrap_or(0.0);
    let by_days = r_days > r_count;
    let ratio = r_count.max(r_days);
    let state = if ratio >= 1.0 {
        DueState::Overdue
    } else if ratio * 100.0 >= soon_pct {
        DueState::Soon
    } else {
        DueState::Ok
    };
    Progress {
        value: round_to(value, 2),
        limit: round_to(limit, 2),
        days: round_to(days, 1),
        days_limit: days_limit.map(|l| round_to(l, 1)),
        ratio,
        by_days,
        state,
    }
}

/// One snooze as a fraction of the interval: 5 %, but at least one whole job for
/// job-count tasks.
pub fn snooze_step(task: &MaintenanceTask) -> f64 {
    if task.basis == Basis::Jobs && task.interval > 0.0 {
        let jobs = (task.interval * SNOOZE_STEP).ceil().max(1.0);
        jobs / task.interval
    } else {
        SNOOZE_STEP
    }
}

/// Short number for titles: whole numbers without decimals, else one decimal.
pub fn fmt_num(v: f64) -> String {
    if (v - v.round()).abs() < 0.05 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.1}")
    }
}

/// Attention title: "Belt tension overdue on HellV0 · 212 / 200 print hours".
pub fn attention_title(task: &MaintenanceTask, printer: &str, p: &Progress) -> String {
    let tail = match (p.by_days, p.days_limit) {
        (true, Some(l)) => format!("{} / {} days", fmt_num(p.days), fmt_num(l)),
        _ => format!(
            "{} / {} {}",
            fmt_num(p.value),
            fmt_num(p.limit),
            task.basis.unit()
        ),
    };
    format!("{} overdue on {printer} · {tail}", task.name)
}

/// A ready-made task offered when a printer is created.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct TaskTemplate {
    pub key: &'static str,
    pub name: &'static str,
    pub notes: &'static str,
    pub basis: Basis,
    pub interval: f64,
    #[ts(type = "number | null")]
    pub every_days: Option<i64>,
}

pub const TEMPLATES: &[TaskTemplate] = &[
    TaskTemplate {
        key: "belt_tension",
        name: "Belt tension",
        notes: "Check and re-tension the motion belts.",
        basis: Basis::Hours,
        interval: 200.0,
        every_days: None,
    },
    TaskTemplate {
        key: "nozzle_change",
        name: "Nozzle change",
        notes: "Swap the nozzle; brass wears fastest with abrasive filament.",
        basis: Basis::Grams,
        interval: 5000.0,
        every_days: None,
    },
    TaskTemplate {
        key: "lube_rails",
        name: "Lubricate rails",
        notes: "Clean and grease linear rails and lead screws.",
        basis: Basis::Hours,
        interval: 300.0,
        every_days: Some(180),
    },
    TaskTemplate {
        key: "clean_bed",
        name: "Clean bed",
        notes: "Wash the build plate (dish soap, or IPA between washes).",
        basis: Basis::Jobs,
        interval: 20.0,
        every_days: None,
    },
    TaskTemplate {
        key: "check_fans",
        name: "Check fans",
        notes: "Clear dust and listen for worn bearings on hotend and part fans.",
        basis: Basis::Hours,
        interval: 500.0,
        every_days: Some(90),
    },
];

pub fn template(key: &str) -> Option<&'static TaskTemplate> {
    TEMPLATES.iter().find(|t| t.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(basis: Basis, interval: f64) -> MaintenanceTask {
        MaintenanceTask {
            id: 1,
            printer_id: 1,
            name: "Belt tension".into(),
            notes: None,
            basis,
            interval,
            every_days: None,
            snooze: 0.0,
            since: None,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        }
    }

    fn c(hours: f64, grams: f64, jobs: i64) -> Counters {
        Counters { hours, grams, jobs }
    }

    #[test]
    fn thresholds() {
        let t = task(Basis::Hours, 200.0);
        let now = Utc::now();
        assert_eq!(
            progress(&t, &c(100.0, 0.0, 0), now, 90.0).state,
            DueState::Ok
        );
        assert_eq!(
            progress(&t, &c(179.9, 0.0, 0), now, 90.0).state,
            DueState::Ok
        );
        assert_eq!(
            progress(&t, &c(180.0, 0.0, 0), now, 90.0).state,
            DueState::Soon
        );
        assert_eq!(
            progress(&t, &c(199.9, 0.0, 0), now, 90.0).state,
            DueState::Soon
        );
        assert_eq!(
            progress(&t, &c(200.0, 0.0, 0), now, 90.0).state,
            DueState::Overdue
        );
        assert_eq!(
            progress(&t, &c(212.0, 0.0, 0), now, 90.0).state,
            DueState::Overdue
        );
        // A lower threshold widens "soon".
        assert_eq!(
            progress(&t, &c(150.0, 0.0, 0), now, 75.0).state,
            DueState::Soon
        );
    }

    #[test]
    fn basis_picks_its_counter() {
        let now = Utc::now();
        let counters = c(1.0, 600.0, 3);
        assert_eq!(
            progress(&task(Basis::Grams, 500.0), &counters, now, 90.0).state,
            DueState::Overdue
        );
        assert_eq!(
            progress(&task(Basis::Jobs, 3.0), &counters, now, 90.0).state,
            DueState::Overdue
        );
        assert_eq!(
            progress(&task(Basis::Hours, 10.0), &counters, now, 90.0).state,
            DueState::Ok
        );
    }

    #[test]
    fn days_interval_and_snooze() {
        let now = Utc::now();
        let mut t = task(Basis::Hours, 200.0);
        t.every_days = Some(30);
        t.since = Some((now - chrono::Duration::days(31)).to_rfc3339());
        let p = progress(&t, &c(10.0, 0.0, 0), now, 90.0);
        assert_eq!(p.state, DueState::Overdue);
        assert!(p.by_days);
        assert_eq!(
            attention_title(&t, "HellV0", &p),
            "Belt tension overdue on HellV0 · 31 / 30 days"
        );
        // Snooze stretches both intervals.
        t.snooze = 0.1;
        assert_ne!(
            progress(&t, &c(10.0, 0.0, 0), now, 90.0).state,
            DueState::Overdue
        );
    }

    #[test]
    fn snooze_step_is_a_whole_job() {
        assert_eq!(snooze_step(&task(Basis::Jobs, 2.0)), 0.5);
        assert_eq!(snooze_step(&task(Basis::Jobs, 100.0)), 0.05);
        assert_eq!(snooze_step(&task(Basis::Hours, 200.0)), SNOOZE_STEP);
    }

    #[test]
    fn title_matches_brief() {
        let t = task(Basis::Hours, 200.0);
        let p = progress(&t, &c(212.0, 0.0, 0), Utc::now(), 90.0);
        assert_eq!(
            attention_title(&t, "HellV0", &p),
            "Belt tension overdue on HellV0 · 212 / 200 print hours"
        );
    }

    #[test]
    fn form_validation() {
        let mut f = TaskForm {
            printer_id: Some(1),
            name: "Clean bed".into(),
            notes: None,
            basis: Basis::Jobs,
            interval: 20.0,
            every_days: None,
            start_now: false,
        };
        assert!(f.validate().is_ok());
        f.interval = 2.5;
        assert!(f.validate().is_err());
        f.interval = 0.0;
        assert!(f.validate().is_err());
        f.interval = 20.0;
        f.every_days = Some(0);
        assert!(f.validate().is_err());
    }
}
