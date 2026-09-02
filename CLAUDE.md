# open_oura — repo guide for agents

Library base for cloud-free access to the Oura ring: the BLE protocol, the event
decoders, the sync drain, on-ring pairing, SQLite storage, and the ecore-ported metric
algorithms. **No app code lives here.** The web dashboard, the iOS app (SwiftUI +
UniFFI `oura-core`), `oura-summary`, and the model runners live in
https://github.com/Th0rgal/open_health, which consumes these crates as git dependencies.

## Crates (fetch → interpret → apply; see `docs/architecture.md`)

- `oura-protocol`: pure framing, request builders, auth crypto, event decoders,
  `RingGeneration`.
- `oura-link`: `Transport` trait, `OuraClient` (auth, sync drain, features, live
  streams), `pair::{probe, pair}`; btleplug behind the `ble` feature; a scripted
  `transport::mock::MockTransport` behind the `mock` feature.
- `oura-analysis`: `ported::*` (ecore ports, cite the source address) and `beats`
  (generic beat/window utilities).
- `oura-store`: SQLite with `PRAGMA user_version` migrations (`SCHEMA_VERSION`).
- `oura-cli`: the `oura` binary (`scan`, `probe`, `pair`, `info`, `sync`, …).

## Rules

- Hard cutover: no backward-compat shims. Move the logic, delete the old path, and update
  the docs in the same change.
- Reusable protocol/library work goes here. Product, UI, model, and summary work goes to
  `open_health`. When `open_health` needs a new query or algorithm, add it here first and
  bump the git `rev` there.
- Local iteration: `open_health/Cargo.toml` carries
  `[patch."https://github.com/Th0rgal/open_oura"]` entries that point at
  `../open_oura/crates/*`. Remove them before a release and bump the `rev` pins instead.
- Never commit keys (`*.key`), databases, captures, or model files (see `.gitignore`).
- Where a change goes: a new decoder → `oura-protocol::events` plus a test with captured
  bytes; a new BLE command → a `protocol` builder plus an `OuraClient` method; a new metric
  → `oura-analysis` plus `docs/algorithms/`; a new table or query → `oura-store` with a
  `SCHEMA_VERSION` bump, a migration step, and a test.
- `oura-link` tests do no I/O: script `MockTransport` (`on`, `on_sequence`, `on_prefix`).
- Use the `1.93.0` toolchain (`cargo +1.93.0 …`); the default `stable` on this machine is
  too old for the lock file.
- Before you finish: `cargo +1.93.0 test --workspace` and
  `cargo +1.93.0 build -p oura-link --no-default-features` (the iOS build) must pass.

## Writing

Write docs and commit messages in ASD-STE100 Simplified Technical English: short
sentences, active voice, one meaning per word.
