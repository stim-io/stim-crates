#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformKind {
    Darwin,
    Linux,
    Windows,
    Other,
}

pub fn platform_kind() -> PlatformKind {
    match std::env::consts::OS {
        "macos" => PlatformKind::Darwin,
        "linux" => PlatformKind::Linux,
        "windows" => PlatformKind::Windows,
        _ => PlatformKind::Other,
    }
}

pub fn arch() -> &'static str {
    std::env::consts::ARCH
}
