//! Activity: ecore's five contributors with the weights recovered from a trends
//! export (`docs/algorithms/score-weights.md`) and its per-contributor mapper
//! shape (`get_activity_score_raw @ 0x1d5788`: four input breakpoints onto
//! `Y = [0, 25, 95, 100]`, then `round(Σ wᵢ·cᵢ / 100)`).
//!
//! The X-tables did not read back from the binary, and two contributors depend on
//! state the ring does not carry: `Meet daily targets` tracks an adaptive personal
//! calorie goal kept by the app/account, and the training contributors encode a
//! multi-day load. So the breakpoints here are explicit and documented:
//!
//! | contributor | weight | input | breakpoints (worst → best) |
//! | --- | --- | --- | --- |
//! | Move every hour | 33 | inactive periods ≥ 1 h while awake | 8, 4, 1, 0 |
//! | Meet daily targets | 24 | active kcal ÷ daily goal | 0.3, 0.6, 1.0, 1.2 |
//! | Stay active | 17 | inactive hours while awake | 12, 9, 6, 4 |
//! | Training volume | 15 | moderate-or-harder minutes, last 7 days | 0, 60, 150, 250 (WHO 2020) |
//! | Training frequency | 12 | days with ≥ 20 min moderate-or-harder, last 7 | 0, 1, 3, 5 |
//!
//! The daily goal is a profile setting (default 450 active kcal, a common Oura
//! default) rather than Oura's adaptive target. A day still in progress is scored
//! on what it has so far and flagged provisional; the two training contributors
//! are provisional until seven days of history exist.

use super::{combine, ecore_curve, Contributor, Part, Score};

/// Default daily active-calorie goal when the profile does not set one.
pub const DEFAULT_GOAL_KCAL: f64 = 450.0;
/// Days of history the training contributors need before they stop being provisional.
pub const TRAINING_WINDOW_DAYS: usize = 7;

#[derive(Default, Clone, Copy, Debug)]
pub struct DayInput {
    /// Minutes awake with the ring on and metabolic rate below light activity.
    pub inactive_min: Option<f64>,
    /// Runs of at least one hour of such inactivity while awake.
    pub long_inactive_periods: Option<f64>,
    /// Active kcal so far today.
    pub active_kcal: Option<f64>,
    /// The daily goal in active kcal.
    pub goal_kcal: f64,
    /// Minutes at moderate intensity or harder (≥ 3 MET) over the past 7 days.
    pub week_moderate_min: Option<f64>,
    /// Days in the past 7 with at least 20 such minutes.
    pub week_active_days: Option<f64>,
    /// Days of activity history behind the two weekly contributors.
    pub history_days: usize,
    /// False while the day is still running (today).
    pub day_complete: bool,
}

fn part(
    key: &'static str,
    name: &'static str,
    score: f64,
    raw_weight: f64,
    value: Option<f64>,
    unit: &'static str,
    provisional: bool,
    source: &'static str,
) -> Part {
    Part {
        contributor: Contributor { key, name, score, weight: 0.0, value, unit, provisional, source },
        raw_weight,
    }
}

/// Score a day. `None` when nothing scoreable was supplied.
pub fn score(input: DayInput) -> Option<Score> {
    let mut parts: Vec<Part> = Vec::new();
    let partial = !input.day_complete;
    let ecore = "ecore mapper shape, documented breakpoints";

    if let Some(periods) = input.long_inactive_periods {
        parts.push(part(
            "move_every_hour",
            "Move every hour",
            ecore_curve(periods, [8.0, 4.0, 1.0, 0.0]),
            33.0,
            Some(periods),
            "long sits",
            partial,
            ecore,
        ));
    }

    if let Some(kcal) = input.active_kcal {
        let goal = if input.goal_kcal > 0.0 { input.goal_kcal } else { DEFAULT_GOAL_KCAL };
        parts.push(part(
            "meet_daily_targets",
            "Meet daily goal",
            ecore_curve(kcal / goal, [0.3, 0.6, 1.0, 1.2]),
            24.0,
            Some(kcal.round()),
            "kcal",
            partial,
            "Active kcal vs your daily goal",
        ));
    }

    if let Some(inactive) = input.inactive_min {
        parts.push(part(
            "stay_active",
            "Stay active",
            ecore_curve(inactive / 60.0, [12.0, 9.0, 6.0, 4.0]),
            17.0,
            Some((inactive / 60.0 * 10.0).round() / 10.0),
            "h inactive",
            partial,
            ecore,
        ));
    }

    let training_provisional = input.history_days < TRAINING_WINDOW_DAYS;
    if let Some(minutes) = input.week_moderate_min {
        parts.push(part(
            "training_volume",
            "Training volume",
            ecore_curve(minutes, [0.0, 60.0, 150.0, 250.0]),
            15.0,
            Some(minutes.round()),
            "min / 7 days",
            training_provisional,
            "WHO 2020: 150 min moderate activity per week",
        ));
    }

    if let Some(days) = input.week_active_days {
        parts.push(part(
            "training_frequency",
            "Training frequency",
            ecore_curve(days, [0.0, 1.0, 3.0, 5.0]),
            12.0,
            Some(days),
            "days / 7",
            training_provisional,
            ecore,
        ));
    }

    combine(parts, "ecore contributors + recovered weights, documented breakpoints")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active_day() -> DayInput {
        DayInput {
            inactive_min: Some(4.5 * 60.0),
            long_inactive_periods: Some(0.0),
            active_kcal: Some(520.0),
            goal_kcal: 450.0,
            week_moderate_min: Some(180.0),
            week_active_days: Some(5.0),
            history_days: 10,
            day_complete: true,
        }
    }

    #[test]
    fn an_active_complete_day_scores_high() {
        let s = score(active_day()).unwrap();
        assert!(s.score >= 95.0, "{s:?}");
        assert!(!s.provisional);
        assert_eq!(s.contributors.len(), 5);
    }

    #[test]
    fn a_desk_day_scores_low() {
        let s = score(DayInput {
            inactive_min: Some(11.0 * 60.0),
            long_inactive_periods: Some(7.0),
            active_kcal: Some(120.0),
            week_moderate_min: Some(10.0),
            week_active_days: Some(0.0),
            ..active_day()
        })
        .unwrap();
        assert!(s.score <= 20.0, "{s:?}");
    }

    #[test]
    fn today_and_a_short_history_are_provisional() {
        let s = score(DayInput { day_complete: false, history_days: 2, ..active_day() }).unwrap();
        assert!(s.provisional);
        assert!(s.contributors.iter().all(|c| c.provisional));
        assert!(score(DayInput::default()).is_none());
    }

    #[test]
    fn the_goal_defaults_when_unset() {
        let a = score(DayInput { goal_kcal: 0.0, ..active_day() }).unwrap();
        let b = score(DayInput { goal_kcal: DEFAULT_GOAL_KCAL, ..active_day() }).unwrap();
        assert_eq!(a.score, b.score);
    }
}
