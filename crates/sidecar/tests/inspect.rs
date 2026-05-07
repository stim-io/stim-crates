use stim_sidecar::{
    identity::{SidecarMode, SidecarStamp, SOURCE_TOOL_STIM_DEV},
    inspect::{EmptyInspectPayload, LiveInspectEnvelope, LiveInspectState},
};

#[test]
fn inspect_is_live_fact() {
    let envelope = LiveInspectEnvelope {
        inspected_at: "2026-04-27T00:00:00Z".into(),
        stamp: SidecarStamp {
            app: "controller".into(),
            namespace: "default".into(),
            mode: SidecarMode::Dev,
            source: SOURCE_TOOL_STIM_DEV.into(),
        },
        role: Some("controller-runtime".into()),
        instance_id: Some("controller-1".into()),
        state: LiveInspectState::Ready,
        detail: None,
        payload: EmptyInspectPayload {},
    };

    assert_eq!(envelope.state, LiveInspectState::Ready);
    assert_eq!(envelope.stamp.mode, SidecarMode::Dev);
}
