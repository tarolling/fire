use std::collections::HashMap;
use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum FireError {
    #[error("workspace config not found (searched from {0} to filesystem root)")]
    WorkspaceNotFound(PathBuf),

    #[error("project '{0}' not found in workspace")]
    ProjectNotFound(String),

    #[error("unsupported language: {0}")]
    UnsupportedLanguage(String),

    #[error("toolchain '{tool}' not found on PATH")]
    ToolchainNotFound { tool: String },

    #[error("command failed: {command} (exit code {code})")]
    CommandFailed { command: String, code: i32 },

    #[error("{0}")]
    Config(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, FireError>;

// ---------------------------------------------------------------------------
// Language
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Rust,
    Python,
    TypeScript,
    Cpp,
    C,
    Java,
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Language::Rust => write!(f, "rust"),
            Language::Python => write!(f, "python"),
            Language::TypeScript => write!(f, "typescript"),
            Language::Cpp => write!(f, "cpp"),
            Language::C => write!(f, "c"),
            Language::Java => write!(f, "java"),
        }
    }
}

impl std::str::FromStr for Language {
    type Err = FireError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "rust" | "rs" => Ok(Language::Rust),
            "python" | "py" => Ok(Language::Python),
            "typescript" | "ts" => Ok(Language::TypeScript),
            "cpp" | "c++" | "cxx" => Ok(Language::Cpp),
            "c" => Ok(Language::C),
            "java" | "jvm" => Ok(Language::Java),
            _ => Err(FireError::UnsupportedLanguage(s.to_string())),
        }
    }
}

// ---------------------------------------------------------------------------
// Config types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ToolchainConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_build_flags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_run_flags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_test_flags: Vec<String>,
}

impl ToolchainConfig {
    pub fn is_default(&self) -> bool {
        self.version.is_none()
            && self.extra_build_flags.is_empty()
            && self.extra_run_flags.is_empty()
            && self.extra_test_flags.is_empty()
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ProjectConfig {
    pub name: String,
    pub path: String,
    pub language: Language,
    #[serde(default, skip_serializing_if = "ToolchainConfig::is_default")]
    pub toolchain: ToolchainConfig,
}

// ---------------------------------------------------------------------------
// ProjectContext — runtime context passed to backends
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ProjectContext {
    pub project: ProjectConfig,
    pub workspace_root: PathBuf,
    pub project_root: PathBuf,
    pub passthrough_flags: Vec<String>,
}

impl ProjectContext {
    pub fn new(project: ProjectConfig, workspace_root: &Path, passthrough_flags: Vec<String>) -> Self {
        let project_root = workspace_root.join(&project.path);
        Self {
            project,
            workspace_root: workspace_root.to_path_buf(),
            project_root,
            passthrough_flags,
        }
    }

    pub fn build_flags(&self) -> Vec<String> {
        let mut flags = self.project.toolchain.extra_build_flags.clone();
        flags.extend(self.passthrough_flags.clone());
        flags
    }

    pub fn run_flags(&self) -> Vec<String> {
        let mut flags = self.project.toolchain.extra_run_flags.clone();
        flags.extend(self.passthrough_flags.clone());
        flags
    }

    pub fn test_flags(&self) -> Vec<String> {
        let mut flags = self.project.toolchain.extra_test_flags.clone();
        flags.extend(self.passthrough_flags.clone());
        flags
    }

    pub fn fire_dir(&self) -> PathBuf {
        self.workspace_root.join(".fire")
    }

    pub fn bin_dir(&self) -> PathBuf {
        self.fire_dir().join("bin")
    }
}

// ---------------------------------------------------------------------------
// Tool environment — project-local toolchain isolation
// ---------------------------------------------------------------------------

/// Set up process environment so that tools installed in `.fire/` are found
/// first. Call once after discovering the workspace root.
pub fn setup_tool_env(workspace_root: &Path) {
    let fire_dir = workspace_root.join(".fire");
    let bin_dir = fire_dir.join("bin");
    let cargo_bin = fire_dir.join("cargo").join("bin");
    let node_bin = fire_dir.join("node").join("bin");
    let cmake_bin = fire_dir.join("cmake").join("bin");
    let vcpkg_dir = fire_dir.join("vcpkg");
    let jdk_bin = fire_dir.join("jdk").join("bin");
    let gradle_bin = fire_dir.join("gradle").join("bin");

    let _ = std::fs::create_dir_all(&bin_dir);

    // Prepend .fire tool directories to PATH
    if let Some(current_path) = std::env::var_os("PATH") {
        let extra_dirs = [&bin_dir, &cargo_bin, &node_bin, &cmake_bin, &vcpkg_dir, &jdk_bin, &gradle_bin];
        let all_paths = extra_dirs.iter()
            .map(|d| d.to_path_buf())
            .chain(std::env::split_paths(&current_path));
        if let Ok(new_path) = std::env::join_paths(all_paths) {
            // SAFETY: called once at startup before any threads are spawned.
            unsafe { std::env::set_var("PATH", &new_path) };
        }
    }

    // Only isolate Rust toolchain if fire previously installed one here.
    // Otherwise, use whatever rustup/cargo the user already has.
    let fire_rustup = fire_dir.join("rustup");
    if fire_rustup.exists() {
        // SAFETY: called once at startup before any threads are spawned.
        unsafe {
            std::env::set_var("RUSTUP_HOME", &fire_rustup);
            std::env::set_var("CARGO_HOME", fire_dir.join("cargo"));
        }
    }
}

// ---------------------------------------------------------------------------
// LanguageBackend trait
// ---------------------------------------------------------------------------

pub trait LanguageBackend: Send + Sync {
    fn name(&self) -> &str;
    fn detect(&self, path: &Path) -> bool;
    fn init(&self, ctx: &ProjectContext) -> Result<()>;
    fn install(&self, ctx: &ProjectContext) -> Result<()>;
    fn add_dep(&self, ctx: &ProjectContext, dep: &str, dev: bool) -> Result<()>;
    fn remove_dep(&self, ctx: &ProjectContext, dep: &str) -> Result<()>;
    fn build(&self, ctx: &ProjectContext) -> Result<()>;
    fn run(&self, ctx: &ProjectContext) -> Result<()>;
    fn test(&self, ctx: &ProjectContext) -> Result<()>;
    fn fmt(&self, ctx: &ProjectContext) -> Result<()>;
    fn lint(&self, ctx: &ProjectContext) -> Result<()>;
    fn clean(&self, ctx: &ProjectContext) -> Result<()>;
    fn ensure_toolchain(&self, ctx: &ProjectContext) -> Result<()>;
}

// ---------------------------------------------------------------------------
// BackendRegistry
// ---------------------------------------------------------------------------

pub struct BackendRegistry {
    backends: HashMap<Language, Box<dyn LanguageBackend>>,
}

impl BackendRegistry {
    pub fn new() -> Self {
        Self {
            backends: HashMap::new(),
        }
    }

    pub fn register(&mut self, language: Language, backend: Box<dyn LanguageBackend>) {
        self.backends.insert(language, backend);
    }

    pub fn get(&self, language: &Language) -> Result<&dyn LanguageBackend> {
        self.backends
            .get(language)
            .map(|b| b.as_ref())
            .ok_or_else(|| FireError::UnsupportedLanguage(language.to_string()))
    }
}

impl Default for BackendRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_from_str() {
        assert_eq!("rust".parse::<Language>().unwrap(), Language::Rust);
        assert_eq!("rs".parse::<Language>().unwrap(), Language::Rust);
        assert_eq!("python".parse::<Language>().unwrap(), Language::Python);
        assert_eq!("py".parse::<Language>().unwrap(), Language::Python);
        assert_eq!("typescript".parse::<Language>().unwrap(), Language::TypeScript);
        assert_eq!("ts".parse::<Language>().unwrap(), Language::TypeScript);
        assert_eq!("cpp".parse::<Language>().unwrap(), Language::Cpp);
        assert_eq!("c++".parse::<Language>().unwrap(), Language::Cpp);
        assert_eq!("cxx".parse::<Language>().unwrap(), Language::Cpp);
        assert_eq!("c".parse::<Language>().unwrap(), Language::C);
        assert_eq!("java".parse::<Language>().unwrap(), Language::Java);
        assert_eq!("jvm".parse::<Language>().unwrap(), Language::Java);
        assert!("haskell".parse::<Language>().is_err());
    }

    #[test]
    fn language_display() {
        assert_eq!(Language::Rust.to_string(), "rust");
        assert_eq!(Language::Python.to_string(), "python");
        assert_eq!(Language::TypeScript.to_string(), "typescript");
    }

    #[test]
    fn toolchain_config_default_is_default() {
        assert!(ToolchainConfig::default().is_default());
    }

    #[test]
    fn project_context_merges_flags() {
        let proj = ProjectConfig {
            name: "api".to_string(),
            path: "projects/api".to_string(),
            language: Language::Rust,
            toolchain: ToolchainConfig {
                version: None,
                extra_build_flags: vec!["--release".to_string()],
                extra_run_flags: vec![],
                extra_test_flags: vec![],
            },
        };
        let ctx = ProjectContext::new(proj, Path::new("/tmp/ws"), vec!["--jobs=4".to_string()]);
        assert_eq!(ctx.build_flags(), vec!["--release", "--jobs=4"]);
        assert_eq!(ctx.run_flags(), vec!["--jobs=4"]);
    }
}
