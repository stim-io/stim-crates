# stim-crates

Reusable Rust platform and sidecar primitives for the stim stack. These crates hold consumer-side primitives; lifecycle and inspect orchestration are consumed through the external `sidecar` CLI.

## Crates

- `stim-platform`: platform facts (paths, processes, networking, environment, locks, OS detection).
- `stim-sidecar`: sidecar identity, namespace, layout, ready-line, stamp, inspect, stamped-process matching.

## Consumers

Consumed via `path` deps from sibling submodules under the `stim.io` workspace root, e.g.:

```toml
stim-platform = { path = "../stim-crates/crates/platform" }
stim-sidecar = { path = "../stim-crates/crates/sidecar" }
```

## Guard

```bash
cargo fmt --all --check
cargo clippy --workspace --locked --all-targets -- -D warnings
cargo test --workspace --locked
```
