use stim_sidecar::{
    identity::SidecarMode,
    process::{command_matches_criteria, env_matches_criteria, StampedProcessCriteria},
};

#[test]
fn matches_stamped_process() {
    let command = concat!(
        "stim-controller ",
        "--sidecar-stamp-app=controller ",
        "--sidecar-stamp-namespace=default ",
        "--sidecar-stamp-mode=dev ",
        "--sidecar-stamp-source=tool:sidecar"
    );
    let criteria = StampedProcessCriteria {
        app: Some("controller".into()),
        namespace: Some("default".into()),
        mode: Some(SidecarMode::Dev),
        source: Some("tool:sidecar".into()),
    };

    assert!(command_matches_criteria(command, &criteria));
}

#[test]
fn rejects_other_namespace() {
    let criteria = StampedProcessCriteria {
        app: Some("controller".into()),
        namespace: Some("other".into()),
        ..StampedProcessCriteria::default()
    };

    assert!(!command_matches_criteria(
        "stim-controller --sidecar-stamp-app=controller --sidecar-stamp-namespace=default",
        &criteria
    ));
}

#[test]
fn matches_env_stamped_process() {
    let env = vec![
        ("STIM_SIDECAR_APP".to_string(), "renderer".to_string()),
        (
            "STIM_SIDECAR_NAMESPACE".to_string(),
            "codex-regression".to_string(),
        ),
        ("STIM_SIDECAR_MODE".to_string(), "dev".to_string()),
        (
            "STIM_SIDECAR_SOURCE".to_string(),
            "tool:sidecar".to_string(),
        ),
    ];
    let criteria = StampedProcessCriteria {
        app: Some("renderer".into()),
        namespace: Some("codex-regression".into()),
        mode: Some(SidecarMode::Dev),
        source: Some("tool:sidecar".into()),
    };

    assert!(env_matches_criteria(&env, &criteria));
}

#[test]
fn rejects_unstamped_env() {
    let env = vec![("STIM_WORKSPACE_ROOT".to_string(), "/tmp/stim".to_string())];

    assert!(!env_matches_criteria(
        &env,
        &StampedProcessCriteria::default()
    ));
}
