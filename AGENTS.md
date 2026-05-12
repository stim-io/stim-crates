# AGENTS

## Purpose

`stim-crates` is the boundary for reusable Rust platform/sidecar libraries shared across the stim stack.

This repo owns business-agnostic primitives: platform facts (paths / processes / networking / environment / locks / OS detection), sidecar identity / namespace / layout / ready-line / stamp / inspect contracts, and stamped-process matching. These primitives are consumed by Rust components in the stack — `stim`, `stim-agents`, and future sidecars — without dragging product semantics across repo boundaries.

This repo does NOT own:

- product CLI tooling or lifecycle orchestration (lives in the external `sidecar` CLI when product-neutral, or in provider-owned inspect events when product-specific).
- product harnesses, scenarios, or chat acceptance (each consumer keeps its own).
- runtime sidecar implementations (each consumer wraps these primitives in its own sidecar binary).

## Core Rules

- Keep crates business-agnostic. No `stim-server` / `santi` / IM-specific code lands here.
- Prefer small focused crates with concrete ownership over a broad utility crate.
- Keep the Cargo workspace minimal. If a new crate is just renaming an existing slice, push back.
- Releases are versioned commits + green guard. External publish flow is deferred until consumers outside this workspace need it.
- Do not let CLI growth back-fill into this repo. External `sidecar` owns lifecycle/inspect routing; this repo only owns consumer-side primitives used by provider binaries.

## Current Crates

- `crates/platform`: platform facts for paths, processes, networking, environment, locks, and OS detection.
- `crates/sidecar`: sidecar identity, namespace, layout, ready-line, stamp, inspect, and stamped-process matching.

## Common Commands

- Run formatter check: `cargo fmt --all --check`
- Run clippy: `cargo clippy --workspace --locked --all-targets -- -D warnings`
- Run tests: `cargo test --workspace --locked`
