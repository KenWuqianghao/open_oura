# Live scores: Sleep, Readiness, Activity (`oura-analysis::scores`)

The three 0–100 daily scores, computed on the device from ring data. No cloud, no
calibration file, no model file.

## What is exact and what is not

Oura's combiner structure and weights are recovered (see
[`score-weights.md`](score-weights.md)): each score is `round(Σ wᵢ·cᵢ / 100)` over a
fixed contributor set. The per-contributor curves are constant tables that do not read
back from `libappecore.so`, and two contributors depend on state the ring does not
carry (an adaptive calorie goal, multi-day training load).

So `oura-analysis::scores` keeps the recovered **contributor sets and weights** and
scores each contributor with an explicit, documented curve. The output carries every
contributor with its applied weight, its input value, and a `provisional` flag, so the
UI can show why a number is what it is.

Expect these to **correlate** with Oura's numbers, not to match them. The sleep score
was validated at r = 0.81 against 661 nights of a trends export. Readiness and
Activity have no such validation yet.

## Sleep (`scores::sleep`)

Literature-based. Ported from Maxxis20's `sleep_score.rs` in open_health (MIT).

| component | weight | source |
| --- | --- | --- |
| Total sleep (or time in bed when there are no stages, flagged provisional) | 30 | Hirshkowitz 2015, NSF duration bands by age |
| Efficiency | 20 | Ohayon 2017, NSF quality consensus |
| Time to fall asleep | 10 | Ohayon 2017 |
| Awake during the night (WASO) | 15 | Ohayon 2017 |
| Awakenings | 5 | Ohayon 2017 (scored leniently: hypnogram arousals ≠ >5 min awakenings) |
| Stage balance (deep %, REM %) | 10 | Boulos 2019; weighted low per de Zambotti 2019 |
| Heart rate & HRV vs your 14-day baseline | 10 | personal z-score |

Components with no input drop out and the weights renormalise.

## Readiness (`scores::readiness`)

ecore's eight contributors and the recovered weights (17/15/15/13/12/10/10/7). Every
baseline-relative contributor uses the wearer's own trailing 14-night window
(`Stats`: mean, SD, count). It is **provisional** below 14 nights and absent below 3.

| contributor | input |
| --- | --- |
| Resting heart rate | lowest night HR, z vs baseline |
| Previous night | last night's sleep score |
| HRV balance | nightly RMSSD, z vs baseline |
| Body temperature | nightly skin temperature minus baseline mean |
| Sleep balance | 14-day mean sleep ÷ personal sleep need |
| Previous day activity | yesterday's active kcal, z vs baseline |
| Recovery index | share of the night after resting HR bottomed out |
| Activity balance | 7-day mean active kcal ÷ 14-day mean |

## Activity (`scores::activity`)

ecore's five contributors, the recovered weights (33/24/17/15/12), and its mapper
shape: four input breakpoints onto `Y = [0, 25, 95, 100]`.

| contributor | input | breakpoints worst → best |
| --- | --- | --- |
| Move every hour | inactive runs ≥ 1 h while awake | 8, 4, 1, 0 |
| Meet daily goal | active kcal ÷ goal (profile `activity_goal_kcal`, default 450) | 0.3, 0.6, 1.0, 1.2 |
| Stay active | inactive hours while awake | 12, 9, 6, 4 |
| Training volume | minutes ≥ 3 MET, last 7 days | 0, 60, 150, 250 (WHO 2020) |
| Training frequency | days with ≥ 20 such minutes, last 7 | 0, 1, 3, 5 |

"Inactive" is a minute below 1.5 MET outside a sleep window. A day still in progress is
scored on what it has and flagged provisional; the training contributors are
provisional below 7 days of history.

## Where it runs

`oura-summary::build_summary` (open_health) feeds the scorers and emits
`scores.days[YYYY-MM-DD] = { sleep, readiness, activity }` keyed by wake date, plus
`scores.latest`. The web dashboard and the iOS app render that block.
