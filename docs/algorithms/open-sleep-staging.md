# Open sleep staging (`oura-analysis::insights::open_sleep`)

`open_sleep` stages a night from the heartbeat intervals with an open model:
`wrn-gru-mesa` from SleepECG (Brunner and Hofer 2023, BSD-3-Clause). SleepECG trained
it on 1971 MESA nights with polysomnography labels. On 1000 SHHS nights it gives an
accuracy of 0.75 and a Cohen's κ of 0.54. The classes are wake, REM and non-REM.
**The model cannot find deep sleep.** It is not a diagnosis.

## Status: in the library, not in use

`oura-summary` does not call this module. The hypnogram comes from Oura's SleepNet
model when the client runs it, else from the ring's own pages (`sleep_phase_data`,
tag `0x5a`). A Gen3 ring scores each sleep of about 2 hours or more itself, with
deep sleep.

A check on 23 sleeps of one Gen3 wearer (age 22, 2026-10-05) compared this model
with the ring's own stages, epoch by epoch, in 3 classes: agreement 0.68, Cohen's
κ 0.30. The model found 27 of the 1107 REM epochs. A different age input made the
result worse. The probable cause is the training data: older adults and chest ECG,
not a young adult and finger PPG. Do not show these stages to a user.

The port stays as a tested base for a model that we train ourselves.

## Inputs

- RR intervals: (time of the beat that ends the interval, interval) in seconds after
  the start of the sleep window. Make them from the `0x60` records of the night with
  `oura_analysis::beats::chained_intervals`, which keeps the intervals of adjacent
  records in sequence.
- The number of 30 s epochs: the window length divided by 30 s.
- The local clock time of the window start, the age and the sex. A missing age or
  sex is allowed.

`stage_night` gives no result for fewer than 2 epochs, or when fewer than half of
the epochs have heart-rate data.

## Features (36 per epoch)

The port follows `sleepecg.extract_features` value for value:

- Window: from 120 s before to 150 s after the epoch start.
- RR intervals out of 0.3 s to 2 s are NaN.
- 26 time-domain features: meanNN, maxNN, minNN, rangeNN, SDNN, RMSSD, SDSD, NN50,
  NN20, pNN50, pNN20, medianNN, madNN, iqrNN, cvNN, cvSD, meanHR, maxHR, minHR,
  stdHR, SD1, SD2, S, SD1/SD2, CSI, CVI.
- 7 frequency-domain features: total power, VLF, LF, LF norm, HF, HF norm, LF/HF.
  The RR series is resampled at 4 Hz with linear interpolation. Each window has
  1080 points. A complete window uses a boxcar periodogram. A window with up to half
  of its points missing uses a Hann window on the valid points, zero-padded to 1080.
  A window with more missing points has no frequency features.
- Metadata: start time (seconds after midnight), age, sex (0 female, 1 male).

Two quirks of the original stay, because the model learned from them:

- pNN50 and pNN20 divide by the longest window of the night, not by the window
  itself (NumPy pads all windows with NaN to one width).
- A value that cannot be computed becomes −1 before the network (the mask value).

For speed, the DFT of a complete window is the sum of its nine 120-sample block
DFTs, each turned by its offset. The values are the same as a direct DFT within
floating-point error.

## Network

Masking (−1) → BatchNorm (ε 0.001) → Dense 64 → ReLU → bidirectional GRU (8 units)
→ bidirectional GRU (8 units) → Dense 4 → softmax over UNDEFINED, NREM, REM, WAKE.
The GRUs use the Keras defaults (gate order z, r, h; `reset_after`). The code picks
the most probable of NREM, REM and WAKE. `confidence` is the mean probability of the
chosen class.

## Weights and verification

`tools/export_sleepecg.py` writes `wrn_gru_mesa.bin` (7380 little-endian f32
values) and `golden.json` (three synthetic nights with SleepECG's own features and
probabilities). The tests check:

- every feature within 1e-7 (relative) of SleepECG, and the same NaN pattern;
- every class probability within 1e-4 of Keras.

To regenerate, install `sleepecg` from its repository, `keras` and `torch`, then run
`KERAS_TORCH_DEVICE=cpu python tools/export_sleepecg.py`.

An 8-hour night (960 epochs) takes about 60 ms on an Apple M-series CPU in a release
build.

## Source

- Brunner C, Hofer F. SleepECG: a Python package for sleep staging based on heart
  rate. J Open Source Softw 2023;8:5411. https://github.com/cbrnr/sleepecg
- Chen X et al. Racial/ethnic differences in sleep disturbances: the Multi-Ethnic
  Study of Atherosclerosis (MESA). Sleep 2015;38:877.
