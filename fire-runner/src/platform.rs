//! Platform detection for cross-platform tool downloads.

/// Operating system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    MacOS,
    Windows,
}

/// CPU architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X64,
    Arm64,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(target_os = "linux") {
            Os::Linux
        } else if cfg!(target_os = "macos") {
            Os::MacOS
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            // Default to Linux for unknown Unix-likes
            Os::Linux
        }
    }
}

impl Arch {
    pub fn current() -> Self {
        if cfg!(target_arch = "aarch64") {
            Arch::Arm64
        } else {
            Arch::X64
        }
    }
}

// ---------------------------------------------------------------------------
// Platform-specific identifiers for download URLs
// ---------------------------------------------------------------------------

/// Node.js platform string (e.g. "linux-x64", "darwin-arm64", "win-x64").
pub fn node_platform() -> &'static str {
    match (Os::current(), Arch::current()) {
        (Os::Linux, Arch::X64) => "linux-x64",
        (Os::Linux, Arch::Arm64) => "linux-arm64",
        (Os::MacOS, Arch::X64) => "darwin-x64",
        (Os::MacOS, Arch::Arm64) => "darwin-arm64",
        (Os::Windows, Arch::X64) => "win-x64",
        (Os::Windows, Arch::Arm64) => "win-arm64",
    }
}

/// Node.js archive extension.
pub fn node_archive_ext() -> &'static str {
    if Os::current() == Os::Windows { "zip" } else { "tar.gz" }
}

/// CMake platform string (e.g. "linux-x86_64", "macos-universal", "windows-x86_64").
pub fn cmake_platform() -> &'static str {
    match (Os::current(), Arch::current()) {
        (Os::Linux, Arch::X64) => "linux-x86_64",
        (Os::Linux, Arch::Arm64) => "linux-aarch64",
        (Os::MacOS, _) => "macos-universal",
        (Os::Windows, Arch::X64) => "windows-x86_64",
        (Os::Windows, Arch::Arm64) => "windows-arm64",
    }
}

/// CMake archive extension.
pub fn cmake_archive_ext() -> &'static str {
    if Os::current() == Os::Windows { "zip" } else { "tar.gz" }
}

/// Ninja platform string (e.g. "ninja-linux", "ninja-mac", "ninja-win").
pub fn ninja_archive_name() -> &'static str {
    match Os::current() {
        Os::Linux => "ninja-linux.zip",
        Os::MacOS => "ninja-mac.zip",
        Os::Windows => "ninja-win.zip",
    }
}

/// JDK (Adoptium Temurin) platform identifiers.
pub fn jdk_os() -> &'static str {
    match Os::current() {
        Os::Linux => "linux",
        Os::MacOS => "mac",
        Os::Windows => "windows",
    }
}

pub fn jdk_arch() -> &'static str {
    match Arch::current() {
        Arch::X64 => "x64",
        Arch::Arm64 => "aarch64",
    }
}

pub fn jdk_archive_ext() -> &'static str {
    if Os::current() == Os::Windows { "zip" } else { "tar.gz" }
}

/// Executable suffix (empty on Unix, ".exe" on Windows).
pub fn exe_suffix() -> &'static str {
    if Os::current() == Os::Windows { ".exe" } else { "" }
}

/// The PATH separator for the current platform.
pub fn path_separator() -> &'static str {
    if Os::current() == Os::Windows { ";" } else { ":" }
}
