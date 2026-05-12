use stim_sidecar::{
    identity::{SidecarMode, SidecarStamp, SOURCE_TOOL_SIDECAR},
    stamp::{
        command_contains_stamp, create_stamp_args, read_stamp, STAMP_APP_FLAG, STAMP_MODE_FLAG,
    },
};

#[test]
fn roundtrips_stamp_args() {
    let stamp = SidecarStamp {
        app: "controller".into(),
        namespace: "default".into(),
        mode: SidecarMode::Dev,
        source: SOURCE_TOOL_SIDECAR.into(),
    };
    let args = create_stamp_args(&stamp);

    assert_eq!(args.len(), 4);
    assert_eq!(read_stamp(&args).unwrap(), stamp);
}

#[test]
fn rejects_runtime_mode_flag() {
    let args = vec![
        format!("{STAMP_APP_FLAG}=controller"),
        "--sidecar-stamp-namespace=default".into(),
        format!("{STAMP_MODE_FLAG}=runtime-mode"),
        "--sidecar-stamp-source=tool:sidecar".into(),
    ];

    assert!(read_stamp(&args).is_err());
}

#[test]
fn matches_split_flags() {
    assert!(command_contains_stamp(
        "stim-controller --sidecar-stamp-app=controller",
        STAMP_APP_FLAG,
        "controller"
    ));
    assert!(command_contains_stamp(
        "stim-controller --sidecar-stamp-app controller",
        STAMP_APP_FLAG,
        "controller"
    ));
}
