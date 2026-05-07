use std::sync::Mutex;

use stim_platform::paths::WORKSPACE_ROOT_ENV;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn workspace_root_env_wins() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = std::env::temp_dir();
    std::env::set_var(WORKSPACE_ROOT_ENV, &temp);

    assert_eq!(
        stim_platform::paths::workspace_root(),
        temp.canonicalize().unwrap()
    );

    std::env::remove_var(WORKSPACE_ROOT_ENV);
}

#[test]
fn dev_root_uses_workspace_root() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp = std::env::temp_dir();
    std::env::set_var(WORKSPACE_ROOT_ENV, &temp);

    assert_eq!(
        stim_platform::paths::dev_root(),
        temp.canonicalize().unwrap().join(".tmp/dev")
    );

    std::env::remove_var(WORKSPACE_ROOT_ENV);
}

#[test]
fn workspace_root_is_absolute() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::remove_var(WORKSPACE_ROOT_ENV);
    let root = stim_platform::paths::workspace_root();

    assert!(root.is_absolute());
    assert!(root.join("Cargo.toml").is_file());
}
