use std::fmt;

use serde::{Deserialize, Serialize};

pub const DEFAULT_NAMESPACE: &str = "default";
pub const SIDECAR_NAMESPACE_ENV: &str = "STIM_SIDECAR_NAMESPACE";
pub const SIDECAR_MODE_ENV: &str = "STIM_SIDECAR_MODE";
pub const SOURCE_APP_PACKAGED: &str = "app:packaged";
pub const SOURCE_APP_TAURI: &str = "app:tauri";
pub const SOURCE_TOOL_STIM_DEV: &str = "tool:stim-dev";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SidecarMode {
    Dev,
    Runtime,
}

impl SidecarMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Runtime => "runtime",
        }
    }
}

impl fmt::Display for SidecarMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::str::FromStr for SidecarMode {
    type Err = ParseSidecarModeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "dev" => Ok(Self::Dev),
            "runtime" => Ok(Self::Runtime),
            _ => Err(ParseSidecarModeError(value.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseSidecarModeError(pub String);

impl fmt::Display for ParseSidecarModeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unsupported sidecar-mode: {}", self.0)
    }
}

impl std::error::Error for ParseSidecarModeError {}

pub fn namespace_or_default(namespace: Option<&str>) -> String {
    namespace
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_NAMESPACE)
        .to_string()
}

pub fn mode_or_default(mode: Option<&str>, default: SidecarMode) -> SidecarMode {
    mode.and_then(|value| value.parse::<SidecarMode>().ok())
        .unwrap_or(default)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidecarStamp {
    pub app: String,
    pub namespace: String,
    pub mode: SidecarMode,
    pub source: String,
}
