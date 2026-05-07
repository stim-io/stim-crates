use std::path::{Path, PathBuf};

use crate::identity::namespace_or_default;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarLayout {
    pub namespace: String,
    pub root: PathBuf,
    pub logs_root: PathBuf,
    pub locks_root: PathBuf,
    pub bridges_root: PathBuf,
}

impl SidecarLayout {
    pub fn new(dev_root: impl AsRef<Path>, namespace: Option<&str>) -> Self {
        let namespace = namespace_or_default(namespace);
        let root = dev_root.as_ref().join(&namespace);

        Self {
            namespace,
            logs_root: root.join("logs"),
            locks_root: root.join("locks"),
            bridges_root: root.join("bridges"),
            root,
        }
    }

    pub fn app_log_path(&self, app: &str) -> PathBuf {
        self.logs_root.join(app).join("latest.log")
    }

    pub fn app_lock_path(&self, app: &str) -> PathBuf {
        self.locks_root.join(format!("{app}.lock"))
    }
}
