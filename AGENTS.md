# AGENTS

## Purpose

`stim-crates` is the boundary for reusable Rust platform/sidecar libraries shared across the stim stack.

This repo owns business-agnostic primitives: platform facts (paths / processes / networking / environment / locks / OS detection), sidecar identity / namespace / layout / ready-line / stamp / inspect contracts, and stamped-process matching. These primitives are consumed by any Rust component in the stack — `stim`, the `stim-dev` CLI, `stim-agents` — without dragging product semantics across repo boundaries.

This repo does NOT own:

- product CLI tooling (lives in `stim-dev`).
- product harnesses, scenarios, or chat acceptance (each consumer keeps its own).
- runtime sidecar implementations (each consumer wraps these primitives in its own sidecar binary).

## Core Rules

- Keep crates business-agnostic. No `stim-server` / `santi` / IM-specific code lands here.
- Prefer small focused crates with concrete ownership over a broad utility crate.
- Keep the Cargo workspace minimal. If a new crate is just renaming an existing slice, push back.
- Releases are versioned commits + green guard. External publish flow is deferred until consumers outside this workspace need it.
- Do not let CLI growth back-fill into this repo. The `stim-dev` repo is the eventual home for the development CLI binary.

## Current Crates

- `crates/platform`: platform facts for paths, processes, networking, environment, locks, and OS detection.
- `crates/sidecar`: sidecar identity, namespace, layout, ready-line, stamp, inspect, and stamped-process matching.

## Common Commands

- Run formatter check: `cargo fmt --all --check`
- Run clippy: `cargo clippy --workspace --locked --all-targets -- -D warnings`
- Run tests: `cargo test --workspace --locked`
