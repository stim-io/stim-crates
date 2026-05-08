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

use std::{
    collections::BTreeMap,
    env, fs,
    fs::OpenOptions,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, ChildStdout, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use crate::{
    identity::{
        SidecarMode, SidecarStamp, SIDECAR_MODE_ENV, SIDECAR_NAMESPACE_ENV, SOURCE_TOOL_STIM_DEV,
    },
    layout::SidecarLayout,
    ready::{wait_for_ready_line, SidecarReadyLine},
    stamp::create_stamp_args,
};

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
    /// Identity for stamps and CLI lookup. Must be unique across all
    /// loaded manifests.
    pub app: String,

    /// How to launch the sidecar process.
    pub launch: LaunchSpec,

    /// Ready-line expectation. If absent, the spawner does not wait for
    /// a ready line and returns the `Child` immediately. Use this for
    /// foreground hosts (Tauri) that don't emit a structured ready line.
    #[serde(default)]
    pub ready: Option<ReadyExpectation>,

    /// Optional override for the stamp source label.
    /// Defaults to `tool:stim-dev`.
    #[serde(default)]
    pub stamp_source: Option<String>,

    /// Whether to also export `SIDECAR_NAMESPACE_ENV` and
    /// `SIDECAR_MODE_ENV` to the spawned process's environment.
    /// CLI args carry the same data; this is for sidecars that read the
    /// stamp via env (e.g. Tauri host).
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
}

/// Ready-line expectation block.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReadyExpectation {
    /// Expected `role` string in the ready line. The spawner validates
    /// the ready line carries this role and a stamp matching the one
    /// the spawner injected.
    pub role: String,
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

/// Choices for stdio handling at spawn time.
#[derive(Debug, Clone, Copy)]
pub enum SpawnStdio {
    /// Inherit stdin/stderr from the parent; pipe stdout for ready-line
    /// reading. Used for foreground / interactive runs.
    Inherit,
    /// Detach from the parent's process group, redirect stdout+stderr
    /// to a per-app log file under `.tmp/dev/<namespace>/logs/<app>/`.
    Detached,
}

/// Caller-supplied options at spawn time.
#[derive(Debug, Clone)]
pub struct SpawnOptions {
    pub namespace: String,
    pub mode: SidecarMode,
    pub stdio: SpawnStdio,
    /// Extra args injected after `launch.args` and before the stamp args.
    pub extra_args: Vec<String>,
    /// Extra env vars layered after `descriptor.env` and stamp env.
    pub extra_env: Vec<(String, String)>,
    /// Optional override for the ready-line wait (defaults to
    /// `DEFAULT_READY_TIMEOUT`).
    pub ready_timeout: Option<Duration>,
}

impl SpawnOptions {
    pub fn new(namespace: impl Into<String>, mode: SidecarMode, stdio: SpawnStdio) -> Self {
        Self {
            namespace: namespace.into(),
            mode,
            stdio,
            extra_args: Vec::new(),
            extra_env: Vec::new(),
            ready_timeout: None,
        }
    }
}

/// Successful spawn result: child handle, optional captured ready-line,
/// optional log path (for Detached stdio).
#[derive(Debug)]
pub struct SpawnResult {
    pub child: Child,
    pub ready: Option<SidecarReadyLine>,
    pub log_path: Option<PathBuf>,
}

#[derive(Debug)]
pub enum SpawnError {
    Io {
        context: String,
        error: std::io::Error,
    },
    UnexpectedReady {
        role: String,
        line: String,
    },
    EarlyExit {
        context: String,
        log_path: Option<PathBuf>,
    },
    Timeout {
        context: String,
        log_path: Option<PathBuf>,
    },
    ReadyChannelMissing {
        context: String,
    },
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpawnError::Io { context, error } => write!(f, "{context}: {error}"),
            SpawnError::UnexpectedReady { role, line } => {
                write!(f, "unexpected ready line for role {role:?}: {line}")
            }
            SpawnError::EarlyExit { context, log_path } => match log_path {
                Some(path) => write!(f, "{context}; see {}", path.display()),
                None => write!(f, "{context}"),
            },
            SpawnError::Timeout { context, log_path } => match log_path {
                Some(path) => write!(
                    f,
                    "{context}: timed out waiting for ready line; see {}",
                    path.display()
                ),
                None => write!(f, "{context}: timed out waiting for ready line"),
            },
            SpawnError::ReadyChannelMissing { context } => {
                write!(f, "{context}: stdout was not piped; cannot read ready line")
            }
        }
    }
}

impl std::error::Error for SpawnError {}

/// Build a `cargo run` command that matches the descriptor + caller options.
///
/// Useful for inspection / unit tests; `spawn_sidecar` calls this internally.
pub fn build_command(
    loaded: &LoadedDescriptor,
    workspace_root: &Path,
    options: &SpawnOptions,
) -> Command {
    let stamp = build_stamp(&loaded.descriptor, &options.namespace, options.mode);
    let stamp_args = create_stamp_args(&stamp);

    let mut cmd = Command::new("cargo");

    match &loaded.descriptor.launch {
        LaunchSpec::Cargo {
            manifest_path,
            package,
            bin,
            cargo_args,
            args,
        } => {
            let abs_manifest = loaded.manifest_dir.join(manifest_path);
            cmd.arg("run").arg("--manifest-path").arg(&abs_manifest);
            if let Some(pkg) = package {
                cmd.arg("-p").arg(pkg);
            }
            if let Some(b) = bin {
                cmd.arg("--bin").arg(b);
            }
            for flag in cargo_args {
                cmd.arg(flag);
            }
            cmd.arg("--");
            for arg in args {
                cmd.arg(arg);
            }
            for arg in &options.extra_args {
                cmd.arg(arg);
            }
            for arg in stamp_args {
                cmd.arg(arg);
            }
        }
    }

    let cwd = match &loaded.descriptor.cwd {
        Some(rel) => loaded.manifest_dir.join(rel),
        None => workspace_root.to_path_buf(),
    };
    cmd.current_dir(cwd);

    for (key, value) in &loaded.descriptor.env {
        cmd.env(key, value);
    }
    if loaded.descriptor.stamp_via_env {
        cmd.env(SIDECAR_NAMESPACE_ENV, &options.namespace);
        cmd.env(SIDECAR_MODE_ENV, options.mode.as_str());
    }
    for (key, value) in &options.extra_env {
        cmd.env(key, value);
    }

    cmd
}

/// Build the stamp this descriptor will inject into the spawned sidecar.
pub fn build_stamp(
    descriptor: &SidecarDescriptor,
    namespace: &str,
    mode: SidecarMode,
) -> SidecarStamp {
    SidecarStamp {
        app: descriptor.app.clone(),
        namespace: namespace.to_string(),
        mode,
        source: descriptor
            .stamp_source
            .clone()
            .unwrap_or_else(|| SOURCE_TOOL_STIM_DEV.to_string()),
    }
}

/// Spawn a sidecar described by `loaded`, applying stamp injection,
/// optional ready-line wait, and optional detached stdio with log
/// redirect.
pub fn spawn_sidecar(
    loaded: &LoadedDescriptor,
    workspace_root: &Path,
    options: &SpawnOptions,
) -> Result<SpawnResult, SpawnError> {
    let mut cmd = build_command(loaded, workspace_root, options);

    let log_path = match options.stdio {
        SpawnStdio::Inherit => {
            cmd.stdin(Stdio::inherit())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit());
            None
        }
        SpawnStdio::Detached => {
            cmd.stdin(Stdio::null());
            detach_process_group(&mut cmd);
            Some(redirect_detached_output(
                &mut cmd,
                workspace_root,
                &options.namespace,
                &loaded.descriptor.app,
            )?)
        }
    };

    let mut child = cmd.spawn().map_err(|error| SpawnError::Io {
        context: format!("failed to spawn {}", loaded.descriptor.app),
        error,
    })?;

    let ready = match (&loaded.descriptor.ready, options.stdio) {
        (Some(expectation), SpawnStdio::Inherit) => {
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| SpawnError::ReadyChannelMissing {
                    context: format!("{} ready check", loaded.descriptor.app),
                })?;
            let line = read_ready_from_stdout(stdout, options)?;
            validate_ready_line(
                loaded,
                &line,
                &options.namespace,
                options.mode,
                &expectation.role,
            )?;
            Some(line)
        }
        (Some(expectation), SpawnStdio::Detached) => {
            let path = log_path
                .as_deref()
                .expect("detached spawn must have produced a log path");
            let line = read_ready_from_log(&mut child, path, options)?;
            validate_ready_line(
                loaded,
                &line,
                &options.namespace,
                options.mode,
                &expectation.role,
            )?;
            Some(line)
        }
        (None, _) => None,
    };

    Ok(SpawnResult {
        child,
        ready,
        log_path,
    })
}

fn read_ready_from_stdout(
    stdout: ChildStdout,
    options: &SpawnOptions,
) -> Result<SidecarReadyLine, SpawnError> {
    let timeout = options.ready_timeout.unwrap_or(DEFAULT_READY_TIMEOUT);
    wait_for_ready_line(stdout, timeout).map_err(|error| SpawnError::EarlyExit {
        context: format!("ready-line wait failed: {error}"),
        log_path: None,
    })
}

fn read_ready_from_log(
    child: &mut Child,
    log_path: &Path,
    options: &SpawnOptions,
) -> Result<SidecarReadyLine, SpawnError> {
    let started = Instant::now();
    let timeout = options.ready_timeout.unwrap_or(DEFAULT_READY_TIMEOUT);
    loop {
        if let Some(line) = peek_ready_in_log(log_path)? {
            return Ok(line);
        }
        if let Some(status) = child.try_wait().map_err(|error| SpawnError::Io {
            context: "checking child status".to_string(),
            error,
        })? {
            return Err(SpawnError::EarlyExit {
                context: format!("sidecar exited before ready with status {status}"),
                log_path: Some(log_path.to_path_buf()),
            });
        }
        if started.elapsed() >= timeout {
            return Err(SpawnError::Timeout {
                context: "sidecar".to_string(),
                log_path: Some(log_path.to_path_buf()),
            });
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn peek_ready_in_log(log_path: &Path) -> Result<Option<SidecarReadyLine>, SpawnError> {
    let file = match fs::File::open(log_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(SpawnError::Io {
                context: format!("failed to open log {}", log_path.display()),
                error,
            })
        }
    };
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|error| SpawnError::Io {
            context: format!("failed to read log {}", log_path.display()),
            error,
        })?;
        if let Ok(parsed) = serde_json::from_str::<SidecarReadyLine>(line.trim()) {
            return Ok(Some(parsed));
        }
    }
    Ok(None)
}

fn validate_ready_line(
    loaded: &LoadedDescriptor,
    line: &SidecarReadyLine,
    namespace: &str,
    mode: SidecarMode,
    expected_role: &str,
) -> Result<(), SpawnError> {
    let expected_stamp = build_stamp(&loaded.descriptor, namespace, mode);
    if !line.is_ready_line() || line.stamp != expected_stamp || line.role != expected_role {
        return Err(SpawnError::UnexpectedReady {
            role: expected_role.to_string(),
            line: serde_json::to_string(line).unwrap_or_default(),
        });
    }
    Ok(())
}

fn redirect_detached_output(
    cmd: &mut Command,
    workspace_root: &Path,
    namespace: &str,
    app: &str,
) -> Result<PathBuf, SpawnError> {
    let layout = SidecarLayout::new(workspace_root.join(".tmp/dev"), Some(namespace));
    let log_path = layout.app_log_path(app);
    let parent = log_path.parent().ok_or_else(|| SpawnError::Io {
        context: format!("log path has no parent: {}", log_path.display()),
        error: std::io::Error::new(std::io::ErrorKind::InvalidInput, "log path has no parent"),
    })?;
    fs::create_dir_all(parent).map_err(|error| SpawnError::Io {
        context: format!("failed to create {}", parent.display()),
        error,
    })?;
    let log = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)
        .map_err(|error| SpawnError::Io {
            context: format!("failed to open {}", log_path.display()),
            error,
        })?;
    let stderr = log.try_clone().map_err(|error| SpawnError::Io {
        context: format!("failed to clone {}", log_path.display()),
        error,
    })?;
    cmd.stdout(Stdio::from(log)).stderr(Stdio::from(stderr));
    Ok(log_path)
}

fn detach_process_group(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    #[cfg(not(unix))]
    {
        let _ = cmd;
    }
}

// Suppress unused-import warnings for the `Read` trait when the file
// only needs `BufRead`; keeping it in scope gives consumers a stable
// API surface if log peek logic later switches to byte streams.
#[allow(dead_code)]
fn _read_marker(_: &mut dyn Read) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_dir() -> PathBuf {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("tests/fixtures");
        p
    }

    #[test]
    fn parses_minimal_manifest() {
        let toml = r#"
manifest_version = 1
project = "stim-agents"

[[sidecars]]
app = "sidecar"
launch.kind = "cargo"
launch.manifest_path = "apps/sidecar/Cargo.toml"
launch.args = ["serve"]

[sidecars.ready]
role = "agents-runtime"
"#;
        let manifest: SidecarManifest = toml::from_str(toml).expect("parse");
        assert_eq!(manifest.manifest_version, 1);
        assert_eq!(manifest.project, "stim-agents");
        assert_eq!(manifest.sidecars.len(), 1);
        let s = &manifest.sidecars[0];
        assert_eq!(s.app, "sidecar");
        assert_eq!(s.ready.as_ref().unwrap().role, "agents-runtime");
        match &s.launch {
            LaunchSpec::Cargo {
                manifest_path,
                args,
                package,
                bin,
                cargo_args,
            } => {
                assert_eq!(manifest_path, &PathBuf::from("apps/sidecar/Cargo.toml"));
                assert_eq!(args, &vec!["serve".to_string()]);
                assert!(package.is_none());
                assert!(bin.is_none());
                assert!(cargo_args.is_empty());
            }
        }
    }

    #[test]
    fn parses_multi_sidecar_manifest_with_optional_fields() {
        let toml = r#"
manifest_version = 1
project = "stim"

[[sidecars]]
app = "controller"
launch = { kind = "cargo", manifest_path = "Cargo.toml", package = "stim-controller", args = ["serve"] }
ready = { role = "controller-runtime" }

[[sidecars]]
app = "tauri"
stamp_via_env = true
cwd = "apps/tauri/src-tauri"
launch = { kind = "cargo", manifest_path = "apps/tauri/src-tauri/Cargo.toml", cargo_args = ["--no-default-features"] }
"#;
        let manifest: SidecarManifest = toml::from_str(toml).expect("parse");
        assert_eq!(manifest.project, "stim");
        assert_eq!(manifest.sidecars.len(), 2);

        let controller = &manifest.sidecars[0];
        assert_eq!(controller.app, "controller");
        assert!(!controller.stamp_via_env);
        assert!(controller.cwd.is_none());

        let tauri = &manifest.sidecars[1];
        assert_eq!(tauri.app, "tauri");
        assert!(tauri.stamp_via_env);
        assert_eq!(
            tauri.cwd.as_deref(),
            Some(Path::new("apps/tauri/src-tauri"))
        );
        assert!(tauri.ready.is_none());
    }

    #[test]
    fn rejects_duplicate_project_across_manifests() {
        let dir = fixture_dir();
        let manifest_a = dir.join("dup_proj_a.toml");
        let manifest_b = dir.join("dup_proj_b.toml");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &manifest_a,
            r#"
manifest_version = 1
project = "stim"
[[sidecars]]
app = "controller"
launch = { kind = "cargo", manifest_path = "Cargo.toml" }
"#,
        )
        .unwrap();
        fs::write(
            &manifest_b,
            r#"
manifest_version = 1
project = "stim"
[[sidecars]]
app = "renderer"
launch = { kind = "cargo", manifest_path = "Cargo.toml" }
"#,
        )
        .unwrap();

        let err = load_registries(&[manifest_a.clone(), manifest_b.clone()]).unwrap_err();
        match err {
            RegistryError::DuplicateProject { project, .. } => assert_eq!(project, "stim"),
            other => panic!("expected DuplicateProject, got {other:?}"),
        }

        let _ = fs::remove_file(&manifest_a);
        let _ = fs::remove_file(&manifest_b);
    }

    #[test]
    fn rejects_duplicate_app_within_project() {
        let dir = fixture_dir();
        let manifest = dir.join("dup_app.toml");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &manifest,
            r#"
manifest_version = 1
project = "stim"
[[sidecars]]
app = "tauri"
launch = { kind = "cargo", manifest_path = "Cargo.toml" }
[[sidecars]]
app = "tauri"
launch = { kind = "cargo", manifest_path = "Cargo.toml" }
"#,
        )
        .unwrap();

        let err = load_registries(std::slice::from_ref(&manifest)).unwrap_err();
        match err {
            RegistryError::DuplicateApp { project, app, .. } => {
                assert_eq!(project, "stim");
                assert_eq!(app, "tauri");
            }
            other => panic!("expected DuplicateApp, got {other:?}"),
        }

        let _ = fs::remove_file(&manifest);
    }

    #[test]
    fn allows_same_app_name_across_different_projects() {
        let dir = fixture_dir();
        let manifest_stim = dir.join("proj_stim.toml");
        let manifest_agents = dir.join("proj_agents.toml");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &manifest_stim,
            r#"
manifest_version = 1
project = "stim"
[[sidecars]]
app = "tauri"
launch = { kind = "cargo", manifest_path = "Cargo.toml" }
"#,
        )
        .unwrap();
        fs::write(
            &manifest_agents,
            r#"
manifest_version = 1
project = "stim-agents"
[[sidecars]]
app = "tauri"
launch = { kind = "cargo", manifest_path = "Cargo.toml" }
"#,
        )
        .unwrap();

        let loaded = load_registries(&[manifest_stim.clone(), manifest_agents.clone()])
            .expect("project namespace separates app names");
        assert_eq!(loaded.len(), 2);
        assert!(find_descriptor(&loaded, "stim", "tauri").is_some());
        assert!(find_descriptor(&loaded, "stim-agents", "tauri").is_some());
        assert!(find_descriptor(&loaded, "stim", "missing").is_none());

        let stim_descs = descriptors_for_project(&loaded, "stim");
        assert_eq!(stim_descs.len(), 1);
        assert_eq!(stim_descs[0].descriptor.app, "tauri");

        let project_names = projects(&loaded);
        assert_eq!(project_names, vec!["stim", "stim-agents"]);

        let _ = fs::remove_file(&manifest_stim);
        let _ = fs::remove_file(&manifest_agents);
    }

    #[test]
    fn build_command_appends_stamp_args_and_resolves_paths() {
        let descriptor = SidecarDescriptor {
            app: "agents".into(),
            launch: LaunchSpec::Cargo {
                manifest_path: PathBuf::from("Cargo.toml"),
                package: None,
                bin: None,
                cargo_args: vec![],
                args: vec!["serve".to_string()],
            },
            ready: Some(ReadyExpectation {
                role: "agents-runtime".into(),
            }),
            stamp_source: None,
            stamp_via_env: false,
            cwd: None,
            env: Default::default(),
        };

        let manifest_dir = PathBuf::from("/tmp/example/stim-agents");
        let manifest_path = manifest_dir.join("sidecars.toml");
        let loaded = LoadedDescriptor {
            project: "stim-agents".into(),
            descriptor,
            manifest_dir: manifest_dir.clone(),
            manifest_path,
        };

        let workspace_root = Path::new("/tmp/example/stim");
        let options = SpawnOptions::new("design", SidecarMode::Dev, SpawnStdio::Inherit);

        let cmd = build_command(&loaded, workspace_root, &options);
        let args: Vec<String> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "run");
        assert_eq!(args[1], "--manifest-path");
        assert_eq!(args[2], "/tmp/example/stim-agents/Cargo.toml");
        assert_eq!(args[3], "--");
        assert_eq!(args[4], "serve");
        // Remaining args should be stamp args; verify shape.
        assert!(args[5..].iter().any(|a| a == "--stim-stamp-app=agents"));
        assert!(args[5..]
            .iter()
            .any(|a| a == "--stim-stamp-namespace=design"));
        assert_eq!(cmd.get_current_dir(), Some(workspace_root));
    }
}
