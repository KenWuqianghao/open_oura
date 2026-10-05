# Insights: open estimates (`oura-analysis::insights`)

Oura gets these results from models or cloud scores. An independent client cannot
run them. Each module here gives a documented estimate. The functions take values
in and give values out. The caller keeps all state.

Expect these results to follow Oura's results, not to match them. None of them is
a medical diagnosis.

| module | result | method |
| --- | --- | --- |
| `regularity` | Sleep Regularity Index, clock spread, chronotype | Phillips 2017; circular statistics; MCTQ midsleep bands |
| `bedtime` | ideal bedtime window | mean bedtime of the best third of the nights |
| `stress` | minutes in four daytime zones | heart rate and RMSSD against the wearer's daytime reference |
| `resilience` | 0–100 score and level | 14-day balance of stress and recovery |
| `illness` | signs in the night's vital signs | each sign against the wearer's own range |
| `nightsignal` | resting heart-rate alert | NightSignal state machine (Mishra 2022) |
| `open_sleep` | wake / REM / non-REM hypnogram (not in use) | SleepECG model ([open-sleep-staging.md](open-sleep-staging.md)) |
| `cycle` | cycle day, phase, next period | calendar mean and the three-over-six temperature rule |

## Sleep regularity (`regularity`)

The Sleep Regularity Index (SRI) is the chance that the wearer is in the same
state, asleep or awake, at two times 24 hours apart. The scale is −100 to 100.
A value of 100 is the same schedule each day.

- A sleep day is noon to noon, so one night is in one day.
- The index compares the state each 5 minutes for each pair of adjacent days that
  have a main sleep (3 hours or more).
- Fewer than 4 day pairs give no result.
- Bedtime, wake time and midpoint use the circular mean and the circular standard
  deviation. A plain mean is wrong for times on the two sides of midnight.

Chronotype uses the mean midpoint of the main sleep:

| midpoint | chronotype |
| --- | --- |
| before 02:30 | `early` |
| 02:30 to 03:30 | `moderately_early` |
| 03:30 to 04:30 | `intermediate` |
| 04:30 to 05:30 | `moderately_late` |
| 05:30 or later | `late` |

The ring has no record of work days, so all days count. The Munich Chronotype
Questionnaire uses free days only.

## Bedtime guidance (`bedtime`)

1. The usual wake time is the circular mean of the wake times.
2. The best nights are the top third of the nights by sleep score (3 or more).
3. The centre of the window is the circular mean bedtime of the best nights.
4. The latest permitted centre is the usual wake time minus the sleep need minus
   15 minutes. A later centre moves to that time.
5. The window is the centre ± 30 minutes, rounded to 5 minutes.

With fewer than 6 scored nights, the centre is the latest permitted centre
(basis `wake_time`). Fewer than 5 nights give no result.

## Daytime stress (`stress`)

Mental stress raises heart rate and lowers beat-to-beat variability (Kim 2018).

- Input: awake windows with mean heart rate, RMSSD, and MET.
- A window with MET of 1.5 or more has movement. It is not classified.
- The reference is the median and the scaled median absolute deviation
  (1.4826 × MAD) of the sedentary windows of the days before. It needs 48 windows.
- Index = (z of heart rate − z of ln RMSSD) ÷ 2. Without RMSSD, the index is the
  heart rate z-score.

| index | zone |
| --- | --- |
| 1.0 or more | `stressed` |
| 0.0 to 1.0 | `engaged` |
| −1.0 to 0.0 | `relaxed` |
| less than −1.0 | `restored` |

The index is relative. `stressed` means high for this wearer.

## Resilience (`resilience`)

Three parts, each 0–100, from the last 14 days:

| part | weight | input |
| --- | --- | --- |
| Sleep recovery | 40 | 0.6 × sleep score + 0.4 × night HRV z-score curve |
| Daytime recovery | 30 | share of classified daytime in `restored` |
| Stress load | 30 | share of classified daytime in `stressed` (less is better) |

A part with no data drops out and the others share its weight. Fewer than 5 days
with data give no result.

| score | level |
| --- | --- |
| 85 or more | `exceptional` |
| 75 to 84 | `strong` |
| 60 to 74 | `solid` |
| 45 to 59 | `adequate` |
| less than 45 | `limited` |

## Illness check (`illness`)

An infection usually raises resting heart rate, breathing rate and skin
temperature and lowers HRV before symptoms start (Mishra 2020, Smarr 2020).

Each sign of last night is compared with the wearer's range from the nights
before. The range is the mean ± 1.5 standard deviations, with a minimum width.

| sign | out of range | minimum half-width | weight |
| --- | --- | --- | --- |
| `AverageBreath` | high | 1.0 breaths/min | 1.0 |
| `LowestHeartRate` | high | 3 bpm | 1.0 |
| `AverageHrv` | low | 15 % of the mean | 1.0 |
| `TemperatureDeviation` | high | 0.3 °C | 1.5 |

A value more than one half-width outside the range counts 1.5 times. The sum of
the weights gives the status: 2.0 or more is `MAJOR_SIGNS`, 1.0 or more is
`MINOR_SIGNS`. A range needs 7 nights of history.

The output has the field names and the status values of Oura's Symptom Radar
model, and `basis: "rules"`.

## NightSignal (`nightsignal`)

NightSignal is the overnight resting-heart-rate alert of the Stanford wearables
study (Mishra 2022). The port follows the reference code
(`StanfordBioinformatics/wearable-infection`, Apache-2.0). It gives the same alerts
as the reference on the reference's Fitbit sample (197 days, 27 alerts).

1. The resting heart rate of a date is the integer mean of the heart-rate samples
   from 00:00 to 06:59 local time. The reference uses minutes with zero steps.
   `oura-summary` uses the ring's heart rate during sleep (`hrv_event`).
2. A single missing date between two dates with data gets their integer mean.
3. The baseline of a date is the integer median of this and all earlier dates.
4. A date with 4 bpm or more above its baseline is a red candidate. A date with
   3 bpm or more above it is a yellow candidate.
5. A date is `red` when it and the date before are red candidates. It is `yellow`
   when they are yellow candidates and the date is not red. Else it is `green`.

The first dates have a short baseline. Infection, stress, alcohol, travel and hard
training can all raise the resting heart rate. An alert is not a diagnosis.

`oura-summary` adds the result to the `illness` object as `nightsignal`, next to the
rule-based check or the model's result.

## Cycle estimate (`cycle`)

- Cycle length: the mean of the last 6 complete cycles of 21 to 45 days. The
  default is 28 days.
- Ovulation: the next period minus 14 days.
- Fertile days: 5 days before ovulation to 1 day after.
- Temperature: ovulation is confirmed when 3 nights in sequence are 0.2 °C or
  more above the highest of the 6 nights before them (Marshall 1968). The day
  before the rise is the ovulation day.

Do not use this estimate for contraception.

## Sources

- Kim H-G et al. Stress and heart rate variability. Psychiatry Investig 2018;15:235.
- Marshall J. A field trial of the basal-body-temperature method. Br Med J 1968;1:803.
- Mishra T et al. Pre-symptomatic detection of COVID-19 from smartwatch data. Nat Biomed Eng 2020;4:1208.
- Mishra T et al. Real-time alerting system for COVID-19 and other stress events using wearable data. Nat Med 2022;28:164.
- Phillips AJK et al. Irregular sleep/wake patterns. Sci Rep 2017;7:3216.
- Roenneberg T et al. Epidemiology of the human circadian clock. Sleep Med Rev 2007;11:429.
- Smarr BL et al. Feasibility of continuous fever monitoring. Sci Rep 2020;10:21640.
