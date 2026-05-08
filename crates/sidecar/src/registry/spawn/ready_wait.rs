use std::{
    fs,
    io::{BufRead, BufReader},
    path::Path,
    process::{Child, ChildStdout},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::{
    identity::SidecarMode,
    ready::{wait_for_ready_line, SidecarReadyLine},
    registry::{LoadedDescriptor, DEFAULT_READY_TIMEOUT},
};

use super::{build_stamp, SpawnError, SpawnOptions};

pub(super) fn read_ready_from_stdout(
    stdout: ChildStdout,
    options: &SpawnOptions,
) -> Result<SidecarReadyLine, SpawnError> {
    let timeout = options.ready_timeout.unwrap_or(DEFAULT_READY_TIMEOUT);
    wait_for_ready_line(stdout, timeout).map_err(|error| SpawnError::EarlyExit {
        context: format!("ready-line wait failed: {error}"),
        log_path: None,
    })
}

/// Compile a stdout-pattern regex once and return a synthesized
/// ready line whose `endpoint` is capture group 1 of the first
/// matching stdout line (after stripping ANSI escapes). Used for
/// shell sidecars (vite, etc.) that don't speak our native
/// ready-line protocol but log their listening URL on startup.
pub(super) fn synthesize_ready_from_pattern(
    stdout: ChildStdout,
    pattern: &str,
    loaded: &LoadedDescriptor,
    options: &SpawnOptions,
    role: &str,
) -> Result<SidecarReadyLine, SpawnError> {
    let regex = regex::Regex::new(pattern).map_err(|error| SpawnError::EarlyExit {
        context: format!("invalid stdout_pattern {pattern:?}: {error}"),
        log_path: None,
    })?;
    let timeout = options.ready_timeout.unwrap_or(DEFAULT_READY_TIMEOUT);
    let started = Instant::now();
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let raw = line.map_err(|error| SpawnError::Io {
            context: "reading sidecar stdout".to_string(),
            error,
        })?;
        if let Some(endpoint) = crate::stdout::extract_endpoint(&raw, &regex) {
            return Ok(synthesized_ready_line(loaded, options, role, endpoint));
        }
        if started.elapsed() >= timeout {
            return Err(SpawnError::Timeout {
                context: format!("{} stdout pattern", loaded.descriptor.app),
                log_path: None,
            });
        }
    }
    Err(SpawnError::EarlyExit {
        context: format!(
            "{} closed stdout before matching pattern {pattern:?}",
            loaded.descriptor.app
        ),
        log_path: None,
    })
}

pub(super) fn synthesize_log_pattern(
    child: &mut Child,
    log_path: &Path,
    pattern: &str,
    loaded: &LoadedDescriptor,
    options: &SpawnOptions,
    role: &str,
) -> Result<SidecarReadyLine, SpawnError> {
    let regex = regex::Regex::new(pattern).map_err(|error| SpawnError::EarlyExit {
        context: format!("invalid stdout_pattern {pattern:?}: {error}"),
        log_path: Some(log_path.to_path_buf()),
    })?;
    let started = Instant::now();
    let timeout = options.ready_timeout.unwrap_or(DEFAULT_READY_TIMEOUT);
    loop {
        if let Some(endpoint) = peek_pattern_in_log(log_path, &regex)? {
            return Ok(synthesized_ready_line(loaded, options, role, endpoint));
        }
        if let Some(status) = child.try_wait().map_err(|error| SpawnError::Io {
            context: "checking child status".to_string(),
            error,
        })? {
            return Err(SpawnError::EarlyExit {
                context: format!("sidecar exited before matching pattern with status {status}"),
                log_path: Some(log_path.to_path_buf()),
            });
        }
        if started.elapsed() >= timeout {
            return Err(SpawnError::Timeout {
                context: format!("{} stdout pattern", loaded.descriptor.app),
                log_path: Some(log_path.to_path_buf()),
            });
        }
        thread::sleep(Duration::from_millis(100));
    }
}

pub(super) fn read_ready_from_log(
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

pub(super) fn validate_ready_line(
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

fn peek_pattern_in_log(
    log_path: &Path,
    regex: &regex::Regex,
) -> Result<Option<String>, SpawnError> {
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
        if let Some(endpoint) = crate::stdout::extract_endpoint(&line, regex) {
            return Ok(Some(endpoint));
        }
    }
    Ok(None)
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

fn synthesized_ready_line(
    loaded: &LoadedDescriptor,
    options: &SpawnOptions,
    role: &str,
    endpoint: String,
) -> SidecarReadyLine {
    let stamp = build_stamp(&loaded.descriptor, &options.namespace, options.mode);
    SidecarReadyLine::new(
        stamp,
        role.to_string(),
        format!(
            "{}-{}",
            loaded.descriptor.app,
            std::process::id().rem_euclid(1_000_000)
        ),
        Some(endpoint),
        timestamp_now(),
    )
}

fn timestamp_now() -> String {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch");
    format!("{}-{:03}", duration.as_secs(), duration.subsec_millis())
}
