use std::path::{Path, PathBuf};

pub const WORKSPACE_ROOT_ENV: &str = "STIM_WORKSPACE_ROOT";

pub fn workspace_root() -> PathBuf {
    if let Some(root) = workspace_root_from_env() {
        return root;
    }
    if let Some(root) = workspace_root_from_cwd() {
        return root;
    }
    manifest_workspace_root()
}

pub fn dev_root() -> PathBuf {
    workspace_root().join(".tmp/dev")
}

fn workspace_root_from_env() -> Option<PathBuf> {
    let root = std::env::var_os(WORKSPACE_ROOT_ENV)?;
    Some(
        PathBuf::from(root)
            .canonicalize()
            .expect("failed to resolve STIM_WORKSPACE_ROOT"),
    )
}

fn workspace_root_from_cwd() -> Option<PathBuf> {
    let current = std::env::current_dir().ok()?;
    find_workspace_root(&current)
}

fn manifest_workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("failed to resolve manifest workspace root")
}

fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    for ancestor in start.ancestors() {
        if is_workspace_root(ancestor) {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

fn is_workspace_root(path: &Path) -> bool {
    path.join(".git").exists() && path.join("Cargo.toml").is_file()
}
