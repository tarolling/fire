use std::path::Path;
use std::process::{Command, Stdio};

use fire_core::{FireError, Result};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub mod platform;

// ---------------------------------------------------------------------------
// CommandRunner — thin wrapper around std::process::Command
// ---------------------------------------------------------------------------

pub struct CommandRunner;

impl CommandRunner {
    /// Run a command with inherited stdio (output streams to terminal).
    pub fn run(program: &str, args: &[&str], cwd: &Path) -> Result<()> {
        which::which(program).map_err(|_| FireError::ToolchainNotFound {
            tool: program.to_string(),
        })?;

        let status = Command::new(program)
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()?;

        if status.success() {
            Ok(())
        } else {
            Err(FireError::CommandFailed {
                command: format!("{} {}", program, args.join(" ")),
                code: status.code().unwrap_or(-1),
            })
        }
    }

    /// Run a command, appending extra flags after the base args.
    pub fn run_with_flags(
        program: &str,
        base_args: &[&str],
        extra_flags: &[String],
        cwd: &Path,
    ) -> Result<()> {
        let mut all_args: Vec<&str> = base_args.to_vec();
        let flag_refs: Vec<&str> = extra_flags.iter().map(|s| s.as_str()).collect();
        all_args.extend(flag_refs);
        Self::run(program, &all_args, cwd)
    }

    /// Run a command and capture stdout.
    pub fn run_capture(program: &str, args: &[&str], cwd: &Path) -> Result<String> {
        which::which(program).map_err(|_| FireError::ToolchainNotFound {
            tool: program.to_string(),
        })?;

        let output = Command::new(program)
            .args(args)
            .current_dir(cwd)
            .output()?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(FireError::CommandFailed {
                command: format!("{} {}", program, args.join(" ")),
                code: output.status.code().unwrap_or(-1),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check that a tool exists on PATH, returning a helpful error if not.
pub fn check_tool(name: &str, install_hint: &str) -> Result<()> {
    which::which(name).map_err(|_| FireError::ToolchainNotFound {
        tool: format!("{} ({})", name, install_hint),
    })?;
    Ok(())
}

/// Returns true if the named tool exists on PATH.
pub fn tool_exists(name: &str) -> bool {
    which::which(name).is_ok()
}

// ---------------------------------------------------------------------------
// Install helpers — download and set up tool binaries
// ---------------------------------------------------------------------------

/// Run a shell script. Uses `sh -c` on Unix, `cmd /C` on Windows.
pub fn run_shell(script: &str, cwd: &Path) -> Result<()> {
    let status = if cfg!(target_os = "windows") {
        Command::new("cmd")
            .args(["/C", script])
            .current_dir(cwd)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()?
    } else {
        Command::new("sh")
            .args(["-c", script])
            .current_dir(cwd)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()?
    };

    if status.success() {
        Ok(())
    } else {
        Err(FireError::CommandFailed {
            command: format!("sh -c '{}'", &script[..script.len().min(80)]),
            code: status.code().unwrap_or(-1),
        })
    }
}

/// Download a file from `url` to `dest` using curl.
pub fn download_file(url: &str, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let dest_str = dest.to_string_lossy();
    let cwd = dest.parent().unwrap_or(Path::new("."));
    CommandRunner::run("curl", &["-fsSL", "-o", &dest_str, url], cwd)
}

/// Download a tarball/zip and extract it into `dest_dir`, stripping leading
/// path components.
pub fn download_and_extract(url: &str, dest_dir: &Path, strip_components: u32) -> Result<()> {
    std::fs::create_dir_all(dest_dir)?;

    if cfg!(target_os = "windows") {
        // On Windows, download to a temp file then extract with tar (available on Win10+)
        let tmp = dest_dir.join("__download.tmp");
        download_file(url, &tmp)?;
        let script = format!(
            "tar -xf \"{}\" --strip-components={} -C \"{}\"",
            tmp.display(),
            strip_components,
            dest_dir.display()
        );
        run_shell(&script, dest_dir)?;
        let _ = std::fs::remove_file(&tmp);
    } else {
        let script = format!(
            "curl -fsSL '{}' | tar xz --strip-components={} -C '{}'",
            url,
            strip_components,
            dest_dir.display()
        );
        run_shell(&script, dest_dir)?;
    }
    Ok(())
}

/// Mark a file as executable (Unix only).
#[cfg(unix)]
pub fn make_executable(path: &Path) -> Result<()> {
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
pub fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}
