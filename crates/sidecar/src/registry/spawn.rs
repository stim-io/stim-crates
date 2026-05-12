mod ready_wait;

use std::{
    fs,
    fs::OpenOptions,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

use crate::{
    identity::{
        SidecarMode, SidecarStamp, SIDECAR_APP_ENV, SIDECAR_MODE_ENV, SIDECAR_NAMESPACE_ENV,
        SIDECAR_SOURCE_ENV, SOURCE_TOOL_SIDECAR,
    },
    layout::SidecarLayout,
    ready::SidecarReadyLine,
    stamp::create_stamp_args,
};

use super::{LaunchSpec, LoadedDescriptor, SidecarDescriptor};
use ready_wait::{
    read_ready_from_log, read_ready_from_stdout, synthesize_log_pattern,
    synthesize_ready_from_pattern, validate_ready_line,
};

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
    // When `stamp_via_env` is set, the sidecar reads the stamp
    // exclusively from environment variables — appending stamp
    // flags on argv breaks strict CLI parsers (e.g. vite's CAC
    // rejects unknown options). When false, stamp flows on argv.
    let stamp_args = if loaded.descriptor.stamp_via_env {
        Vec::new()
    } else {
        create_stamp_args(&stamp)
    };

    let mut cmd = match &loaded.descriptor.launch {
        LaunchSpec::Cargo { .. } => Command::new("cargo"),
        LaunchSpec::Shell { command, .. } => Command::new(command),
    };

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
        LaunchSpec::Shell { args, .. } => {
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
        cmd.env(SIDECAR_APP_ENV, &stamp.app);
        cmd.env(SIDECAR_NAMESPACE_ENV, &options.namespace);
        cmd.env(SIDECAR_MODE_ENV, options.mode.as_str());
        cmd.env(SIDECAR_SOURCE_ENV, &stamp.source);
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
            .unwrap_or_else(|| SOURCE_TOOL_SIDECAR.to_string()),
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
            let line = if let Some(pattern) = expectation.stdout_pattern.as_deref() {
                synthesize_ready_from_pattern(stdout, pattern, loaded, options, &expectation.role)?
            } else {
                let line = read_ready_from_stdout(stdout, options)?;
                validate_ready_line(
                    loaded,
                    &line,
                    &options.namespace,
                    options.mode,
                    &expectation.role,
                )?;
                line
            };
            Some(line)
        }
        (Some(expectation), SpawnStdio::Detached) => {
            let path = log_path
                .as_deref()
                .expect("detached spawn must have produced a log path");
            let line = if let Some(pattern) = expectation.stdout_pattern.as_deref() {
                synthesize_log_pattern(
                    &mut child,
                    path,
                    pattern,
                    loaded,
                    options,
                    &expectation.role,
                )?
            } else {
                let line = read_ready_from_log(&mut child, path, options)?;
                validate_ready_line(
                    loaded,
                    &line,
                    &options.namespace,
                    options.mode,
                    &expectation.role,
                )?;
                line
            };
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
