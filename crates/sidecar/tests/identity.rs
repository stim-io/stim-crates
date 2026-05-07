use stim_sidecar::identity::{mode_or_default, namespace_or_default, SidecarMode};

#[test]
fn mode_accepts_values() {
    assert_eq!("dev".parse::<SidecarMode>().unwrap(), SidecarMode::Dev);
    assert_eq!(
        "runtime".parse::<SidecarMode>().unwrap(),
        SidecarMode::Runtime
    );
    assert!("prod".parse::<SidecarMode>().is_err());
    assert!("runtime-mode".parse::<SidecarMode>().is_err());
}

#[test]
fn namespace_defaults() {
    assert_eq!(namespace_or_default(None), "default");
    assert_eq!(namespace_or_default(Some("")), "default");
    assert_eq!(namespace_or_default(Some(" local ")), "local");
}

#[test]
fn mode_defaults_when_invalid() {
    assert_eq!(mode_or_default(None, SidecarMode::Dev), SidecarMode::Dev);
    assert_eq!(
        mode_or_default(Some("runtime"), SidecarMode::Dev),
        SidecarMode::Runtime
    );
    assert_eq!(
        mode_or_default(Some("runtime-mode"), SidecarMode::Dev),
        SidecarMode::Dev
    );
}
