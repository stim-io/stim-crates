use stim_sidecar::{
    identity::SidecarMode,
    process::{command_matches_criteria, StampedProcessCriteria},
};

#[test]
fn matches_stamped_process() {
    let command = concat!(
        "stim-controller ",
        "--stim-stamp-app=controller ",
        "--stim-stamp-namespace=default ",
        "--stim-stamp-mode=dev ",
        "--stim-stamp-source=tool:stim-dev"
    );
    let criteria = StampedProcessCriteria {
        app: Some("controller".into()),
        namespace: Some("default".into()),
        mode: Some(SidecarMode::Dev),
        source: Some("tool:stim-dev".into()),
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
        "stim-controller --stim-stamp-app=controller --stim-stamp-namespace=default",
        &criteria
    ));
}
