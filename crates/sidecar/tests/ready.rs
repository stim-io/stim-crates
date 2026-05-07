use std::{io::Cursor, time::Duration};

use stim_sidecar::{
    identity::{SidecarMode, SidecarStamp, SOURCE_APP_PACKAGED},
    ready::{wait_for_ready_line, SidecarReadyLine, READY_LINE_KIND},
};

#[test]
fn ready_line_has_kind() {
    let line = SidecarReadyLine::new(
        SidecarStamp {
            app: "controller".into(),
            namespace: "default".into(),
            mode: SidecarMode::Runtime,
            source: SOURCE_APP_PACKAGED.into(),
        },
        "controller-runtime".into(),
        "controller-1".into(),
        Some("http://127.0.0.1:43123".into()),
        "2026-04-27T00:00:00Z".into(),
    );

    assert_eq!(line.kind, READY_LINE_KIND);
    assert!(line.is_ready_line());
    assert_eq!(line.endpoint.as_deref(), Some("http://127.0.0.1:43123"));

    let value = serde_json::to_value(&line).unwrap();
    assert!(value.get("stamp").is_some());
    assert!(value.get("identity").is_none());
    assert_eq!(value["role"], "controller-runtime");
    assert_eq!(value["instance_id"], "controller-1");
}

#[test]
fn waits_for_ready_line() {
    let content = concat!(
        "compile noise\n",
        "{\"kind\":\"stim-sidecar-ready\",\"stamp\":{\"app\":\"controller\",\"namespace\":\"default\",\"mode\":\"runtime\",\"source\":\"app:packaged\"},\"role\":\"controller-runtime\",\"instance_id\":\"controller-1\",\"endpoint\":\"http://127.0.0.1:43123\",\"ready_at\":\"2026-04-27T00:00:00Z\"}\n"
    );
    let ready = wait_for_ready_line(Cursor::new(content), Duration::from_secs(1)).unwrap();

    assert_eq!(ready.stamp.app, "controller");
    assert_eq!(ready.role, "controller-runtime");
}
