# open_oura — repo guide for agents

Library base for cloud-free access to the Oura ring: the BLE protocol, the event
decoders, the sync drain, on-ring pairing, SQLite storage, and the ecore-ported metric
algorithms. **No app code lives here.** The web dashboard, the iOS app (SwiftUI +
UniFFI `oura-core`), `oura-summary`, and the model runners live in
https://github.com/KenWuqianghao/open_health, which consumes these crates as git dependencies.

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

<!-- gitnexus:start -->
# GitNexus — Code Intelligence

This project is indexed by GitNexus as **open_oura** (1372 symbols, 3299 relationships, 106 execution flows).

> Index stale? Run `node .gitnexus/run.cjs analyze --index-only` from the project root — it auto-selects an available runner. No `.gitnexus/run.cjs` yet? Bootstrap with `npx`, `bunx`, or `pnpm dlx` — e.g. `bunx gitnexus@latest analyze` (npm 11 npx crash; #1939).

## Always Do

- **MUST run impact before editing.** Use `impact({target: "symbolName", direction: "upstream"})` or `node .gitnexus/run.cjs impact "symbolName" --direction upstream --repo .`; report callers, processes, and risk. Never substitute grep for graph analysis.
- **MUST analyze graph changes before committing.** Use `detect_changes({scope: "all"})` (MCP) or `node .gitnexus/run.cjs detect-changes --scope all --repo .` (CLI fallback). `partial: true` or `truncated: true` is not a clean check — a zero means unseen, not unaffected; re-run it. For regression review: `detect_changes({scope: "compare", base_ref: "main"})` or `node .gitnexus/run.cjs detect-changes --scope compare --base-ref "main" --repo .`.
- MUST warn on HIGH/CRITICAL `risk` pre-edit; never use `riskSharedAxes` to waive a HIGH/CRITICAL `risk` warning. Compare File/symbol: MCP File omits axes; Graph-RAG expands File.
- **MUST treat `risk: UNKNOWN` as unresolved, not as low.** An empty caller set is not evidence the symbol is unused — it can also mean the callers are not resolvable by the index (plain-object property access, dynamic dispatch, cross-language calls). `impact` pairs `UNKNOWN` with a `riskNote` saying so. Confirm with a text search before treating the symbol as safe to change or delete; do not proceed on the strength of a zero.
- **MUST use `query({search_query: "concept"})` for concepts/flows, `context({name: "symbolName"})` for a named symbol, or `impact` for blast radius, on read-only callers, dependencies, imports, or execution flow.** Graph first; text search only for empty/`UNKNOWN`/literals.
- For security review, `explain({target: "fileOrSymbol"})` lists taint findings (source→sink flows; needs `analyze --pdg`).

## Never Do

- NEVER edit a function, class, or method before MCP/CLI impact analysis.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis, and never read `UNKNOWN` as an all-clear — it means the walk could not answer, which is the one verdict that requires confirming by other means.
- NEVER rename symbols with find-and-replace — use `rename` which understands the call graph.
- NEVER commit before MCP/CLI graph change analysis.

## Resources

| Resource | Use for |
| --- | --- |
| `gitnexus://repo/open_oura/context` | Codebase overview, check index freshness |
| `gitnexus://repo/open_oura/clusters` | All functional areas |
| `gitnexus://repo/open_oura/processes` | All execution flows |
| `gitnexus://repo/open_oura/process/{name}` | Step-by-step execution trace |

## CLI

| Task | Read this skill file |
| --- | --- |
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/gitnexus-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/gitnexus-debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/gitnexus-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.claude/skills/gitnexus-guide/SKILL.md` |
| Index, status, clean, wiki CLI commands | `.claude/skills/gitnexus-cli/SKILL.md` |

<!-- gitnexus:end -->
