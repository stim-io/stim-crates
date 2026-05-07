use std::str::FromStr;

use crate::identity::{SidecarMode, SidecarStamp};

pub const STAMP_APP_FLAG: &str = "--stim-stamp-app";
pub const STAMP_NAMESPACE_FLAG: &str = "--stim-stamp-namespace";
pub const STAMP_MODE_FLAG: &str = "--stim-stamp-mode";
pub const STAMP_SOURCE_FLAG: &str = "--stim-stamp-source";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StampError {
    Missing(&'static str),
    InvalidMode(String),
}

impl std::fmt::Display for StampError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(flag) => write!(formatter, "{flag} is required"),
            Self::InvalidMode(value) => write!(formatter, "invalid sidecar-mode: {value}"),
        }
    }
}

impl std::error::Error for StampError {}

pub fn create_stamp_args(stamp: &SidecarStamp) -> Vec<String> {
    vec![
        format!("{STAMP_APP_FLAG}={}", stamp.app),
        format!("{STAMP_NAMESPACE_FLAG}={}", stamp.namespace),
        format!("{STAMP_MODE_FLAG}={}", stamp.mode),
        format!("{STAMP_SOURCE_FLAG}={}", stamp.source),
    ]
}

pub fn read_flag_value(args: &[String], flag: &'static str) -> Option<String> {
    let inline_prefix = format!("{flag}=");
    let mut index = 0;

    while index < args.len() {
        let arg = &args[index];

        if arg == flag {
            return args.get(index + 1).cloned();
        }

        if let Some(value) = arg.strip_prefix(&inline_prefix) {
            return Some(value.to_string());
        }

        index += 1;
    }

    None
}

fn require_flag(args: &[String], flag: &'static str) -> Result<String, StampError> {
    read_flag_value(args, flag).ok_or(StampError::Missing(flag))
}

pub fn read_stamp(args: &[String]) -> Result<SidecarStamp, StampError> {
    let mode_value = require_flag(args, STAMP_MODE_FLAG)?;
    let mode = SidecarMode::from_str(&mode_value)
        .map_err(|_| StampError::InvalidMode(mode_value.clone()))?;

    Ok(SidecarStamp {
        app: require_flag(args, STAMP_APP_FLAG)?,
        namespace: require_flag(args, STAMP_NAMESPACE_FLAG)?,
        mode,
        source: require_flag(args, STAMP_SOURCE_FLAG)?,
    })
}

pub fn command_contains_stamp(command: &str, flag: &'static str, value: &str) -> bool {
    let inline = format!("{flag}={value}");
    let separated = format!("{flag} {value}");

    command
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .any(|window| window == [flag, value])
        || command.split_whitespace().any(|part| part == inline)
        || command.contains(&separated)
}
