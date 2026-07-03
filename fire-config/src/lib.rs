use std::path::{Path, PathBuf};

use fire_core::{FireError, Result, ProjectConfig};
use serde::{Deserialize, Serialize};

const CONFIG_FILE: &str = "fire.toml";

// ---------------------------------------------------------------------------
// WorkspaceConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WorkspaceConfig {
    pub workspace: WorkspaceMeta,
    #[serde(rename = "project", default, skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<ProjectConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WorkspaceMeta {
    pub name: String,
}

impl WorkspaceConfig {
    pub fn new(name: &str) -> Self {
        Self {
            workspace: WorkspaceMeta {
                name: name.to_string(),
            },
            projects: vec![],
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: WorkspaceConfig = toml::from_str(&content)
            .map_err(|e| FireError::Config(format!("failed to parse {}: {}", path.display(), e)))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        let mut seen = std::collections::HashSet::new();
        for svc in &self.projects {
            if !seen.insert(&svc.name) {
                return Err(FireError::Config(format!(
                    "duplicate project name: {}",
                    svc.name
                )));
            }
        }
        Ok(())
    }

    pub fn find_project(&self, name: &str) -> Result<&ProjectConfig> {
        self.projects
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| FireError::ProjectNotFound(name.to_string()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)
            .map_err(|e| FireError::Config(format!("failed to serialize config: {}", e)))?;
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn add_project(&mut self, project: ProjectConfig) -> Result<()> {
        if self.projects.iter().any(|s| s.name == project.name) {
            return Err(FireError::Config(format!(
                "project '{}' already exists",
                project.name
            )));
        }
        self.projects.push(project);
        Ok(())
    }

    pub fn remove_project(&mut self, name: &str) -> Result<ProjectConfig> {
        let idx = self
            .projects
            .iter()
            .position(|p| p.name == name)
            .ok_or_else(|| FireError::ProjectNotFound(name.to_string()))?;
        Ok(self.projects.remove(idx))
    }
}

// ---------------------------------------------------------------------------
// Workspace discovery
// ---------------------------------------------------------------------------

pub fn discover_workspace(from: &Path) -> Result<(PathBuf, WorkspaceConfig)> {
    let start = if from.is_absolute() {
        from.to_path_buf()
    } else {
        std::env::current_dir()?.join(from)
    };
    let mut current = start.clone();
    loop {
        let config_path = current.join(CONFIG_FILE);
        if config_path.exists() {
            let config = WorkspaceConfig::load(&config_path)?;
            return Ok((current, config));
        }
        if !current.pop() {
            return Err(FireError::WorkspaceNotFound(start));
        }
    }
}

pub fn config_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join(CONFIG_FILE)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use fire_core::Language;

    #[test]
    fn parse_workspace_config() {
        let input = r#"
[workspace]
name = "test-project"

[[project]]
name = "api"
path = "projects/api"
language = "rust"

[[project]]
name = "ml"
path = "projects/ml"
language = "python"

[project.toolchain]
version = "3.12"
extra_build_flags = ["--no-cache"]
"#;
        let config: WorkspaceConfig = toml::from_str(input).unwrap();
        assert_eq!(config.workspace.name, "test-project");
        assert_eq!(config.projects.len(), 2);
        assert_eq!(config.projects[0].name, "api");
        assert_eq!(config.projects[0].language, Language::Rust);
        assert_eq!(config.projects[1].name, "ml");
        assert_eq!(config.projects[1].language, Language::Python);
        assert_eq!(
            config.projects[1].toolchain.version.as_deref(),
            Some("3.12")
        );
        assert_eq!(config.projects[1].toolchain.extra_build_flags, vec!["--no-cache"]);
    }

    #[test]
    fn parse_minimal_config() {
        let input = r#"
[workspace]
name = "empty"
"#;
        let config: WorkspaceConfig = toml::from_str(input).unwrap();
        assert_eq!(config.workspace.name, "empty");
        assert!(config.projects.is_empty());
    }

    #[test]
    fn duplicate_project_names_rejected() {
        let input = r#"
[workspace]
name = "test"

[[project]]
name = "api"
path = "projects/api"
language = "rust"

[[project]]
name = "api"
path = "projects/api2"
language = "python"
"#;
        let config: WorkspaceConfig = toml::from_str(input).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn roundtrip_serialization() {
        let config = WorkspaceConfig::new("roundtrip");
        let serialized = toml::to_string_pretty(&config).unwrap();
        let parsed: WorkspaceConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(parsed.workspace.name, "roundtrip");
        assert!(parsed.projects.is_empty());
    }
}
