use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};

/// Environment variable consumers set to point at registry TOMLs.
/// Colon-separated paths.
pub const REGISTRIES_ENV: &str = "STIM_DEV_REGISTRIES";

/// Default ready-line wait when a descriptor opts into ready validation.
pub const DEFAULT_READY_TIMEOUT: Duration = Duration::from_secs(120);

/// Top-level shape of a `sidecars.toml`.
///
/// `project` identifies the product owner of this manifest (e.g. `stim`
/// or `stim-agents`) and forms the first half of the registry's
/// `(project, app)` lookup key. Both fields are required so the manifest
/// is self-describing — the dev CLI never has to reverse-engineer
/// project identity from a manifest's filesystem path.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SidecarManifest {
    #[serde(default = "default_manifest_version")]
    pub manifest_version: u32,
    pub project: String,
    #[serde(default)]
    pub sidecars: Vec<SidecarDescriptor>,
}

fn default_manifest_version() -> u32 {
    1
}

/// Declarative description of a single sidecar app.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SidecarDescriptor {
    /// Identity for stamps and CLI lookup. Must be unique within
    /// a single manifest's project.
    pub app: String,

    /// How to launch the sidecar process.
    pub launch: LaunchSpec,

    /// Ready-line expectation. If absent, the spawner does not wait for
    /// a ready line and returns the `Child` immediately. Use this for
    /// foreground hosts (Tauri) that don't emit a structured ready line.
    #[serde(default)]
    pub ready: Option<ReadyExpectation>,

    /// Optional override for the stamp source label.
    /// Defaults to `tool:sidecar`.
    #[serde(default)]
    pub stamp_source: Option<String>,

    /// Whether to export the complete sidecar stamp to the spawned
    /// process's environment instead of argv. This is for strict-CLI
    /// sidecars that reject unknown stamp flags.
    #[serde(default)]
    pub stamp_via_env: bool,

    /// Working directory override, relative to the sidecars.toml file.
    /// If absent, the spawner uses its caller-supplied workspace_root.
    /// Use this for sidecars (e.g. Tauri host) whose binary resolves
    /// runtime files relative to a specific directory.
    #[serde(default)]
    pub cwd: Option<PathBuf>,

    /// Static environment variables to set on the spawned process.
    #[serde(default)]
    pub env: BTreeMap<String, String>,

    /// Name of the env var that exposes this sidecar's endpoint to
    /// downstream consumers. When set, the captured ready-line
    /// endpoint is published under this key in the project chain
    /// context, so other sidecars can claim it via `inherits_env`.
    ///
    /// Example: `endpoint_env = "STIM_AGENTS_ENDPOINT"` on the agents
    /// sidecar lets a Tauri host declare
    /// `inherits_env = [{ name = "STIM_AGENTS_ENDPOINT", from = "agents.endpoint" }]`.
    #[serde(default)]
    pub endpoint_env: Option<String>,

    /// Env-var bindings inherited from earlier-launched sidecars in
    /// the chain. Each entry says "set env var `name` to the value of
    /// `from`", where `from` is one of:
    ///   - `<sidecar>.endpoint` — captured ready-line endpoint
    ///   - `<sidecar>.instance_id` — captured ready-line instance id
    ///   - `<sidecar>.endpoint_env` — value of that sidecar's
    ///     `endpoint_env` (transitive convenience).
    ///
    /// Used to model dependencies declaratively: stim's Tauri host
    /// inherits `STIM_AGENTS_ENDPOINT` from `agents.endpoint`, etc.
    #[serde(default)]
    pub inherits_env: Vec<InheritEnvBinding>,
}

/// Ready-line expectation block.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReadyExpectation {
    /// Expected `role` string in the ready line. The spawner validates
    /// the ready line carries this role and a stamp matching the one
    /// the spawner injected.
    pub role: String,

    /// When set, the spawner does NOT expect a structured
    /// `stim-sidecar-ready` JSON line on stdout. Instead it watches
    /// stdout for a line matching this regex, captures group 1 as the
    /// endpoint URL (e.g. vite's `Local: http://127.0.0.1:1234/`),
    /// and synthesizes a ready-line internally. Used for shell
    /// sidecars that don't emit our native protocol.
    #[serde(default)]
    pub stdout_pattern: Option<String>,
}

/// One inherits-env binding.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InheritEnvBinding {
    /// Env var name set on the spawned sidecar.
    pub name: String,
    /// Source path: `<sidecar>.endpoint`, `<sidecar>.instance_id`,
    /// or `<sidecar>.endpoint_env`. Sidecars are looked up in the
    /// current chain context; endpoint paths reference the
    /// captured ready line.
    pub from: String,
}

/// Process launch instructions.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LaunchSpec {
    /// Invoke `cargo run` against a specified manifest. The manifest
    /// path is relative to the sidecars.toml file.
    Cargo {
        manifest_path: PathBuf,
        #[serde(default)]
        package: Option<String>,
        #[serde(default)]
        bin: Option<String>,
        /// Cargo flags placed BEFORE `--` (e.g. `--no-default-features`).
        #[serde(default)]
        cargo_args: Vec<String>,
        /// Binary args placed AFTER `--`, BEFORE the auto-injected stamp args.
        #[serde(default)]
        args: Vec<String>,
    },
    /// Run an arbitrary executable. Used for non-cargo sidecars such
    /// as `pnpm` scripts, vite dev servers, node binaries, or other
    /// platform tools that the consumer treats as part of its
    /// dev-loop process catalog.
    ///
    /// The descriptor's stamp args are appended AFTER `args`, so a
    /// shell command that doesn't accept sidecar-stamp flags should
    /// declare `stamp_via_env = true` instead.
    Shell {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
}

/// A descriptor as loaded from a specific manifest file. Keeps the
/// owning project name + manifest_dir so callers can resolve
/// `(project, app)` lookups and relative paths at spawn time.
#[derive(Debug, Clone)]
pub struct LoadedDescriptor {
    pub project: String,
    pub descriptor: SidecarDescriptor,
    pub manifest_dir: PathBuf,
    pub manifest_path: PathBuf,
}

#[derive(Debug)]
pub enum RegistryError {
    EnvNotSet,
    Io {
        path: PathBuf,
        error: std::io::Error,
    },
    Parse {
        path: PathBuf,
        error: toml::de::Error,
    },
    DuplicateProject {
        project: String,
        paths: Vec<PathBuf>,
    },
    DuplicateApp {
        project: String,
        app: String,
        paths: Vec<PathBuf>,
    },
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::EnvNotSet => {
                write!(f, "{REGISTRIES_ENV} environment variable is not set")
            }
            RegistryError::Io { path, error } => {
                write!(f, "failed to read {}: {error}", path.display())
            }
            RegistryError::Parse { path, error } => {
                write!(f, "failed to parse {}: {error}", path.display())
            }
            RegistryError::DuplicateProject { project, paths } => write!(
                f,
                "project {project:?} is declared by multiple manifests: {}",
                paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            RegistryError::DuplicateApp {
                project,
                app,
                paths,
            } => write!(
                f,
                "sidecar app {app:?} is declared twice within project {project:?}: {}",
                paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

impl std::error::Error for RegistryError {}

/// Parse the registries env var into individual paths.
pub fn registries_from_env() -> Result<Vec<PathBuf>, RegistryError> {
    let var = env::var(REGISTRIES_ENV).map_err(|_| RegistryError::EnvNotSet)?;
    Ok(var
        .split(':')
        .filter(|segment| !segment.is_empty())
        .map(PathBuf::from)
        .collect())
}

/// Read a single sidecars.toml file.
pub fn load_manifest(path: &Path) -> Result<SidecarManifest, RegistryError> {
    let bytes = fs::read_to_string(path).map_err(|error| RegistryError::Io {
        path: path.to_path_buf(),
        error,
    })?;
    toml::from_str(&bytes).map_err(|error| RegistryError::Parse {
        path: path.to_path_buf(),
        error,
    })
}

/// Load all manifests from `STIM_DEV_REGISTRIES` and flatten their
/// descriptors into a single list with project + manifest provenance
/// attached. Returns an error if two manifests claim the same project,
/// or if a single project repeats an app name.
pub fn load_registries(paths: &[PathBuf]) -> Result<Vec<LoadedDescriptor>, RegistryError> {
    let mut loaded = Vec::new();
    let mut projects_seen: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    let mut apps_seen: BTreeMap<(String, String), Vec<PathBuf>> = BTreeMap::new();
    for path in paths {
        let manifest = load_manifest(path)?;
        let project = manifest.project.clone();
        projects_seen
            .entry(project.clone())
            .or_default()
            .push(path.clone());
        let manifest_dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        for descriptor in manifest.sidecars {
            apps_seen
                .entry((project.clone(), descriptor.app.clone()))
                .or_default()
                .push(path.clone());
            loaded.push(LoadedDescriptor {
                project: project.clone(),
                descriptor,
                manifest_dir: manifest_dir.clone(),
                manifest_path: path.clone(),
            });
        }
    }

    for (project, paths) in projects_seen {
        if paths.len() > 1 {
            return Err(RegistryError::DuplicateProject { project, paths });
        }
    }

    for ((project, app), paths) in apps_seen {
        if paths.len() > 1 {
            return Err(RegistryError::DuplicateApp {
                project,
                app,
                paths,
            });
        }
    }

    Ok(loaded)
}

/// Look up a single descriptor by `(project, app)`.
pub fn find_descriptor<'a>(
    loaded: &'a [LoadedDescriptor],
    project: &str,
    app: &str,
) -> Option<&'a LoadedDescriptor> {
    loaded
        .iter()
        .find(|d| d.project == project && d.descriptor.app == app)
}

/// All descriptors that belong to `project`, in manifest declaration order.
pub fn descriptors_for_project<'a>(
    loaded: &'a [LoadedDescriptor],
    project: &str,
) -> Vec<&'a LoadedDescriptor> {
    loaded.iter().filter(|d| d.project == project).collect()
}

/// All registered project names, in their first-seen declaration order.
pub fn projects(loaded: &[LoadedDescriptor]) -> Vec<&str> {
    let mut seen: Vec<&str> = Vec::new();
    for descriptor in loaded {
        let name = descriptor.project.as_str();
        if !seen.contains(&name) {
            seen.push(name);
        }
    }
    seen
}

/// Convenience: read env, load all paths, return descriptors.
pub fn load_registries_from_env() -> Result<Vec<LoadedDescriptor>, RegistryError> {
    let paths = registries_from_env()?;
    load_registries(&paths)
}
