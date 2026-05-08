//! Sidecar registry contract: declarative descriptors that consumers
//! (`stim`, `stim-agents`, ...) publish via a `sidecars.toml` at their
//! repo root. The dev CLI discovers these via the `STIM_DEV_REGISTRIES`
//! environment variable (a colon-separated list of TOML paths) and
//! dispatches generic spawning against them, replacing per-sidecar
//! hardcoded launch logic in the CLI itself.
//!
//! Contract surface lives here so that consumer repos and the dev CLI
//! agree on a single schema; new sidecar apps can join the dev loop by
//! adding a TOML entry without touching the CLI source.

mod manifest;
mod spawn;

pub use manifest::*;
pub use spawn::*;
