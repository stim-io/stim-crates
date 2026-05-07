use crate::{
    identity::SidecarMode,
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

pub fn matching_stamped_processes(
    processes: &[stim_platform::process::ProcessSnapshot],
    criteria: &StampedProcessCriteria,
) -> Vec<stim_platform::process::ProcessSnapshot> {
    processes
        .iter()
        .filter(|process| command_matches_criteria(&process.command, criteria))
        .cloned()
        .collect()
}
