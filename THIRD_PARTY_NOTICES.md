# Third-party notices

This repository contains work derived from the projects below. Their licenses
apply to those parts.

## SleepECG (BSD-3-Clause)

- Parts: `crates/oura-analysis/src/insights/open_sleep/` (the feature extraction,
  ported to Rust) and `wrn_gru_mesa.bin` (the weights of the `wrn-gru-mesa`
  classifier, converted from Keras to f32), plus `tools/export_sleepecg.py`.
- Source: https://github.com/cbrnr/sleepecg
- Copyright (c) 2021, Florian Hofer, Clemens Brunner. All rights reserved.
- License: [`licenses/SleepECG-BSD-3-Clause.txt`](licenses/SleepECG-BSD-3-Clause.txt).

The classifier was trained on data from the Multi-Ethnic Study of Atherosclerosis
(MESA), made available through the National Sleep Research Resource (NSRR). This
repository contains no MESA data.

## NightSignal (Apache-2.0)

- Part: `crates/oura-analysis/src/insights/nightsignal.rs`, a Rust port of
  `nightsignal.py`. Changes: Rust; values in and values out; no files; the alert
  state machine is unchanged.
- Source: https://github.com/StanfordBioinformatics/wearable-infection
- License: [`licenses/Apache-2.0.txt`](licenses/Apache-2.0.txt).
