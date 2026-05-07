use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct FileLock {
    path: PathBuf,
    _file: File,
}

#[derive(Debug)]
pub enum FileLockError {
    Busy { path: PathBuf },
    Io(std::io::Error),
}

impl std::fmt::Display for FileLockError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy { path } => write!(formatter, "lock is busy: {}", path.display()),
            Self::Io(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for FileLockError {}

impl FileLock {
    pub fn acquire(path: impl AsRef<Path>, owner: &str) -> Result<Self, FileLockError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(FileLockError::Io)?;
        }

        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    FileLockError::Busy { path: path.clone() }
                } else {
                    FileLockError::Io(error)
                }
            })?;

        writeln!(file, "{owner}").map_err(FileLockError::Io)?;

        Ok(Self { path, _file: file })
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
