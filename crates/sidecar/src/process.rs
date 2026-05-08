use crate::{
    identity::{
        SidecarMode, SIDECAR_APP_ENV, SIDECAR_MODE_ENV, SIDECAR_NAMESPACE_ENV, SIDECAR_SOURCE_ENV,
    },
    stamp::{
        command_contains_stamp, STAMP_APP_FLAG, STAMP_MODE_FLAG, STAMP_NAMESPACE_FLAG,
        STAMP_SOURCE_FLAG,
    },
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StampedProcessCriteria {
    pub app: Option<String>,
    pub namespace: Option<String>,
    pub mode: Option<SidecarMode>,
    pub source: Option<String>,
}

pub fn command_matches_criteria(command: &str, criteria: &StampedProcessCriteria) -> bool {
    criteria
        .app
        .as_deref()
        .is_none_or(|value| command_contains_stamp(command, STAMP_APP_FLAG, value))
        && criteria
            .namespace
            .as_deref()
            .is_none_or(|value| command_contains_stamp(command, STAMP_NAMESPACE_FLAG, value))
        && criteria
            .mode
            .is_none_or(|value| command_contains_stamp(command, STAMP_MODE_FLAG, value.as_str()))
        && criteria
            .source
            .as_deref()
            .is_none_or(|value| command_contains_stamp(command, STAMP_SOURCE_FLAG, value))
}

pub fn env_matches_criteria(env: &[(String, String)], criteria: &StampedProcessCriteria) -> bool {
    if !env.iter().any(|(key, _)| is_stamp_env_key(key)) {
        return false;
    }

    criteria
        .app
        .as_deref()
        .is_none_or(|value| env_has_value(env, SIDECAR_APP_ENV, value))
        && criteria
            .namespace
            .as_deref()
            .is_none_or(|value| env_has_value(env, SIDECAR_NAMESPACE_ENV, value))
        && criteria
            .mode
            .is_none_or(|value| env_has_value(env, SIDECAR_MODE_ENV, value.as_str()))
        && criteria
            .source
            .as_deref()
            .is_none_or(|value| env_has_value(env, SIDECAR_SOURCE_ENV, value))
}

pub fn matching_stamped_processes(
    processes: &[stim_platform::process::ProcessSnapshot],
    criteria: &StampedProcessCriteria,
) -> Vec<stim_platform::process::ProcessSnapshot> {
    processes
        .iter()
        .filter(|process| process_matches_criteria(process, criteria))
        .cloned()
        .collect()
}

fn process_matches_criteria(
    process: &stim_platform::process::ProcessSnapshot,
    criteria: &StampedProcessCriteria,
) -> bool {
    command_matches_criteria(&process.command, criteria)
        || process_stamp_env(process.pid)
            .as_deref()
            .is_some_and(|env| env_matches_criteria(env, criteria))
}

fn process_stamp_env(pid: u32) -> Option<Vec<(String, String)>> {
    if cfg!(target_os = "windows") {
        return None;
    }

    let output = std::process::Command::new("ps")
        .args(["eww", "-p", &pid.to_string(), "-o", "command="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Some(parse_stamp_ps_env(&stdout))
}

fn parse_stamp_ps_env(output: &str) -> Vec<(String, String)> {
    output
        .split_whitespace()
        .filter_map(|part| {
            let (key, value) = part.split_once('=')?;
            is_stamp_env_key(key).then(|| (key.to_string(), value.to_string()))
        })
        .collect()
}

fn env_has_value(env: &[(String, String)], key: &str, value: &str) -> bool {
    env.iter().any(|(k, v)| k == key && v == value)
}

fn is_stamp_env_key(key: &str) -> bool {
    matches!(
        key,
        SIDECAR_APP_ENV | SIDECAR_NAMESPACE_ENV | SIDECAR_MODE_ENV | SIDECAR_SOURCE_ENV
    )
}
