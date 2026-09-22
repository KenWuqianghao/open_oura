//! `oura-analysis` — the *interpretation (high level)* layer: turning decoded
//! samples into daily metrics and derived insights.
//!
//! [`ported`] holds algorithms **reverse-engineered from Oura's own software** (the
//! on-device `ecore` engine). These aim to reproduce Oura's results and cite the
//! source function `@ address`. [`scores`] holds the live Sleep / Readiness /
//! Activity scores: ecore's contributor sets and recovered weights with explicit,
//! documented curves where the binary's tables could not be read. See
//! `docs/algorithms/`.
//!
//! (Activity-session detection used to live in an `original` namespace of
//! open_oura's own heuristics; app-level activity classification now lives in
//! `open_health`.)

pub mod beats;
pub mod ported;
pub mod scores;
