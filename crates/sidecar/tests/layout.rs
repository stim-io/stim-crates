use stim_sidecar::layout::SidecarLayout;

#[test]
fn layout_uses_namespace() {
    let layout = SidecarLayout::new("/tmp/sidecar", Some("dev-a"));

    assert_eq!(layout.root, std::path::PathBuf::from("/tmp/sidecar/dev-a"));
    assert_eq!(
        layout.app_log_path("controller"),
        std::path::PathBuf::from("/tmp/sidecar/dev-a/logs/controller/latest.log")
    );
    assert_eq!(
        layout.app_lock_path("controller"),
        std::path::PathBuf::from("/tmp/sidecar/dev-a/locks/controller.lock")
    );
}
