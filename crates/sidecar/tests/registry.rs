use std::{
    fs,
    path::{Path, PathBuf},
};

use stim_sidecar::{
    identity::SidecarMode,
    registry::{
        build_command, descriptors_for_project, find_descriptor, load_registries, projects,
        LaunchSpec, LoadedDescriptor, ReadyExpectation, RegistryError, SidecarDescriptor,
        SidecarManifest, SpawnOptions, SpawnStdio,
    },
};

fn fixture_dir() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures");
    path
}

fn loaded_descriptor(descriptor: SidecarDescriptor) -> LoadedDescriptor {
    let manifest_dir = PathBuf::from("/tmp/example/stim-agents");
    let manifest_path = manifest_dir.join("sidecars.toml");
    LoadedDescriptor {
        project: "stim-agents".into(),
        descriptor,
        manifest_dir,
        manifest_path,
    }
}

mod manifest {
    use super::*;

    #[test]
    fn parses_minimal() {
        let toml = r#"
manifest_version = 1
project = "stim-agents"

[[sidecars]]
app = "sidecar"
launch.kind = "cargo"
launch.manifest_path = "apps/sidecar/Cargo.toml"
launch.args = ["serve"]

[sidecars.ready]
role = "agents-runtime"
"#;
        let manifest: SidecarManifest = toml::from_str(toml).expect("parse");
        assert_eq!(manifest.manifest_version, 1);
        assert_eq!(manifest.project, "stim-agents");
        assert_eq!(manifest.sidecars.len(), 1);
        let sidecar = &manifest.sidecars[0];
        assert_eq!(sidecar.app, "sidecar");
        assert_eq!(sidecar.ready.as_ref().unwrap().role, "agents-runtime");
        match &sidecar.launch {
            LaunchSpec::Cargo {
                manifest_path,
                args,
                package,
                bin,
                cargo_args,
            } => {
                assert_eq!(manifest_path, &PathBuf::from("apps/sidecar/Cargo.toml"));
                assert_eq!(args, &vec!["serve".to_string()]);
                assert!(package.is_none());
                assert!(bin.is_none());
                assert!(cargo_args.is_empty());
            }
            other => panic!("expected Cargo launch, got {other:?}"),
        }
    }

    #[test]
    fn parses_optional_fields() {
        let toml = r#"
manifest_version = 1
project = "stim"

[[sidecars]]
app = "controller"
launch = { kind = "cargo", manifest_path = "Cargo.toml", package = "stim-controller", args = ["serve"] }
ready = { role = "controller-runtime" }

[[sidecars]]
app = "tauri"
stamp_via_env = true
cwd = "apps/tauri/src-tauri"
launch = { kind = "cargo", manifest_path = "apps/tauri/src-tauri/Cargo.toml", cargo_args = ["--no-default-features"] }
"#;
        let manifest: SidecarManifest = toml::from_str(toml).expect("parse");
        assert_eq!(manifest.project, "stim");
        assert_eq!(manifest.sidecars.len(), 2);

        let controller = &manifest.sidecars[0];
        assert_eq!(controller.app, "controller");
        assert!(!controller.stamp_via_env);
        assert!(controller.cwd.is_none());

        let tauri = &manifest.sidecars[1];
        assert_eq!(tauri.app, "tauri");
        assert!(tauri.stamp_via_env);
        assert_eq!(
            tauri.cwd.as_deref(),
            Some(Path::new("apps/tauri/src-tauri"))
        );
        assert!(tauri.ready.is_none());
    }
}

mod duplicate_project {
    use super::*;

    #[test]
    fn rejects_across_manifests() {
        let dir = fixture_dir();
        let manifest_a = dir.join("dup_proj_a.toml");
        let manifest_b = dir.join("dup_proj_b.toml");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &manifest_a,
            r#"
manifest_version = 1
project = "stim"
[[sidecars]]
app = "controller"
launch = { kind = "cargo", manifest_path = "Cargo.toml" }
"#,
        )
        .unwrap();
        fs::write(
            &manifest_b,
            r#"
manifest_version = 1
project = "stim"
[[sidecars]]
app = "renderer"
launch = { kind = "cargo", manifest_path = "Cargo.toml" }
"#,
        )
        .unwrap();

        let err = load_registries(&[manifest_a.clone(), manifest_b.clone()]).unwrap_err();
        match err {
            RegistryError::DuplicateProject { project, .. } => assert_eq!(project, "stim"),
            other => panic!("expected DuplicateProject, got {other:?}"),
        }

        let _ = fs::remove_file(&manifest_a);
        let _ = fs::remove_file(&manifest_b);
    }
}

mod duplicate_app {
    use super::*;

    #[test]
    fn rejects_within_project() {
        let dir = fixture_dir();
        let manifest = dir.join("dup_app.toml");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &manifest,
            r#"
manifest_version = 1
project = "stim"
[[sidecars]]
app = "tauri"
launch = { kind = "cargo", manifest_path = "Cargo.toml" }
[[sidecars]]
app = "tauri"
launch = { kind = "cargo", manifest_path = "Cargo.toml" }
"#,
        )
        .unwrap();

        let err = load_registries(std::slice::from_ref(&manifest)).unwrap_err();
        match err {
            RegistryError::DuplicateApp { project, app, .. } => {
                assert_eq!(project, "stim");
                assert_eq!(app, "tauri");
            }
            other => panic!("expected DuplicateApp, got {other:?}"),
        }

        let _ = fs::remove_file(&manifest);
    }
}

mod app_namespace {
    use super::*;

    #[test]
    fn allows_cross_project_names() {
        let dir = fixture_dir();
        let stim = dir.join("proj_stim.toml");
        let agents = dir.join("proj_agents.toml");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &stim,
            r#"
manifest_version = 1
project = "stim"
[[sidecars]]
app = "tauri"
launch = { kind = "cargo", manifest_path = "Cargo.toml" }
"#,
        )
        .unwrap();
        fs::write(
            &agents,
            r#"
manifest_version = 1
project = "stim-agents"
[[sidecars]]
app = "tauri"
launch = { kind = "cargo", manifest_path = "Cargo.toml" }
"#,
        )
        .unwrap();

        let loaded =
            load_registries(&[stim.clone(), agents.clone()]).expect("project namespace is unique");
        assert_eq!(loaded.len(), 2);
        assert!(find_descriptor(&loaded, "stim", "tauri").is_some());
        assert!(find_descriptor(&loaded, "stim-agents", "tauri").is_some());
        assert!(find_descriptor(&loaded, "stim", "missing").is_none());

        let stim_items = descriptors_for_project(&loaded, "stim");
        assert_eq!(stim_items.len(), 1);
        assert_eq!(stim_items[0].descriptor.app, "tauri");

        let names = projects(&loaded);
        assert_eq!(names, vec!["stim", "stim-agents"]);

        let _ = fs::remove_file(&stim);
        let _ = fs::remove_file(&agents);
    }
}

mod cmd_build {
    use super::*;

    #[test]
    fn appends_stamp_args() {
        let descriptor = SidecarDescriptor {
            app: "agents".into(),
            launch: LaunchSpec::Cargo {
                manifest_path: PathBuf::from("Cargo.toml"),
                package: None,
                bin: None,
                cargo_args: vec![],
                args: vec!["serve".to_string()],
            },
            ready: Some(ReadyExpectation {
                role: "agents-runtime".into(),
                stdout_pattern: None,
            }),
            stamp_source: None,
            stamp_via_env: false,
            cwd: None,
            env: Default::default(),
            endpoint_env: None,
            inherits_env: Vec::new(),
        };

        let loaded = loaded_descriptor(descriptor);
        let workspace = Path::new("/tmp/example/stim");
        let options = SpawnOptions::new("design", SidecarMode::Dev, SpawnStdio::Inherit);

        let cmd = build_command(&loaded, workspace, &options);
        let args: Vec<String> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "run");
        assert_eq!(args[1], "--manifest-path");
        assert_eq!(args[2], "/tmp/example/stim-agents/Cargo.toml");
        assert_eq!(args[3], "--");
        assert_eq!(args[4], "serve");
        assert!(args[5..].iter().any(|a| a == "--sidecar-stamp-app=agents"));
        assert!(args[5..]
            .iter()
            .any(|a| a == "--sidecar-stamp-namespace=design"));
        assert_eq!(cmd.get_current_dir(), Some(workspace));
    }

    #[test]
    fn handles_shell_kind() {
        let descriptor = SidecarDescriptor {
            app: "renderer".into(),
            launch: LaunchSpec::Shell {
                command: "pnpm".into(),
                args: vec!["-C".into(), "apps/renderer".into(), "dev".into()],
            },
            ready: None,
            stamp_source: None,
            stamp_via_env: true,
            cwd: None,
            env: Default::default(),
            endpoint_env: None,
            inherits_env: Vec::new(),
        };

        let loaded = loaded_descriptor(descriptor);
        let workspace = Path::new("/tmp/example/stim-agents");
        let options = SpawnOptions::new("default", SidecarMode::Dev, SpawnStdio::Inherit);

        let cmd = build_command(&loaded, workspace, &options);
        assert_eq!(cmd.get_program().to_string_lossy(), "pnpm");
        let args: Vec<String> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "-C");
        assert_eq!(args[1], "apps/renderer");
        assert_eq!(args[2], "dev");
        assert!(
            !args.iter().any(|a| a.starts_with("--sidecar-stamp-")),
            "stamp_via_env should suppress argv stamp flags"
        );

        let envs: Vec<(String, Option<String>)> = cmd
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert!(envs
            .iter()
            .any(|(k, v)| k == "STIM_SIDECAR_APP" && v.as_deref() == Some("renderer")));
        assert!(envs
            .iter()
            .any(|(k, v)| k == "STIM_SIDECAR_NAMESPACE" && v.as_deref() == Some("default")));
        assert!(envs
            .iter()
            .any(|(k, v)| k == "STIM_SIDECAR_MODE" && v.as_deref() == Some("dev")));
        assert!(envs
            .iter()
            .any(|(k, v)| k == "STIM_SIDECAR_SOURCE" && v.as_deref() == Some("tool:sidecar")));
    }
}

mod env_fields {
    use super::*;

    #[test]
    fn parses_inheritance() {
        let toml = r#"
manifest_version = 1
project = "stim"

[[sidecars]]
app = "agents"
launch = { kind = "cargo", manifest_path = "Cargo.toml", args = ["serve"] }
ready = { role = "agents-runtime" }
endpoint_env = "STIM_AGENTS_ENDPOINT"

[[sidecars]]
app = "tauri"
stamp_via_env = true
launch = { kind = "cargo", manifest_path = "apps/tauri/src-tauri/Cargo.toml" }
inherits_env = [
  { name = "STIM_AGENTS_ENDPOINT", from = "agents.endpoint" },
  { name = "STIM_AGENTS_INSTANCE_ID", from = "agents.instance_id" },
]
"#;
        let manifest: SidecarManifest = toml::from_str(toml).expect("parse");
        assert_eq!(manifest.sidecars.len(), 2);

        let agents = &manifest.sidecars[0];
        assert_eq!(agents.endpoint_env.as_deref(), Some("STIM_AGENTS_ENDPOINT"));
        assert!(agents.inherits_env.is_empty());

        let tauri = &manifest.sidecars[1];
        assert_eq!(tauri.inherits_env.len(), 2);
        assert_eq!(tauri.inherits_env[0].name, "STIM_AGENTS_ENDPOINT");
        assert_eq!(tauri.inherits_env[0].from, "agents.endpoint");
    }
}

mod ready_pattern {
    use super::*;

    #[test]
    fn parses_stdout_pattern() {
        let toml = r#"
manifest_version = 1
project = "stim-agents"

[[sidecars]]
app = "renderer"
stamp_via_env = true
launch = { kind = "shell", command = "pnpm", args = ["-C", "apps/renderer", "exec", "vite", "--port", "0"] }
ready = { role = "renderer-delivery", stdout_pattern = "Local:\\s+(http://[^\\s]+)" }
endpoint_env = "STIM_AGENTS_RENDERER_URL"
"#;
        let manifest: SidecarManifest = toml::from_str(toml).expect("parse");
        let renderer = &manifest.sidecars[0];
        let ready = renderer.ready.as_ref().expect("ready block");
        assert_eq!(ready.role, "renderer-delivery");
        assert_eq!(
            ready.stdout_pattern.as_deref(),
            Some(r"Local:\s+(http://[^\s]+)")
        );
        assert_eq!(
            renderer.endpoint_env.as_deref(),
            Some("STIM_AGENTS_RENDERER_URL")
        );
    }
}
