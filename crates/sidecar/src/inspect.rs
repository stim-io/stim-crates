use serde::{Deserialize, Serialize};

use crate::identity::SidecarStamp;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LiveInspectState {
    Ready,
    Degraded,
    Unreachable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveInspectEnvelope<T> {
    pub inspected_at: String,
    pub stamp: SidecarStamp,
    pub role: Option<String>,
    pub instance_id: Option<String>,
    pub state: LiveInspectState,
    pub detail: Option<String>,
    pub payload: T,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmptyInspectPayload {}
