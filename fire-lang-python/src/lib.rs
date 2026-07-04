use std::path::Path;

use fire_core::{LanguageBackend, ProjectContext, Result};
use fire_runner::{CommandRunner, run_shell, tool_exists};

/// Pinned tool versions — bump these explicitly to upgrade.
const UV_VERSION: &str = "0.7.12";

pub struct PythonBackend;

impl PythonBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PythonBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageBackend for PythonBackend {
    fn name(&self) -> &str {
        "python"
    }

    fn detect(&self, path: &Path) -> bool {
        path.join("pyproject.toml").exists()
    }

    fn init(&self, ctx: &ProjectContext) -> Result<()> {
        std::fs::create_dir_all(&ctx.project_root)?;
        // --lib creates src/<name>/ layout with [build-system] in pyproject.toml,
        // which makes the package installable and `python -m <name>` work.
        CommandRunner::run(
            "uv",
            &[
                "init",
                "--lib",
                "--name",
                &ctx.project.name,
                "--vcs",
                "none",
            ],
            &ctx.project_root,
        )?;

        // Add __main__.py so the project is runnable via `python -m <name>`
        let module_name = ctx.project.name.replace('-', "_");
        let pkg_dir = ctx.project_root.join("src").join(&module_name);

        std::fs::write(
            pkg_dir.join("__main__.py"),
            format!(
                "def main() -> None:\n    print(\"Hello from {}!\")\n\n\nif __name__ == \"__main__\":\n    main()\n",
                ctx.project.name
            ),
        )?;

        // Remove flat-layout files that uv init may create
        for name in ["hello.py", "main.py"] {
            let p = ctx.project_root.join(name);
            if p.exists() {
                std::fs::remove_file(&p)?;
            }
        }

        Ok(())
    }

    fn install(&self, ctx: &ProjectContext) -> Result<()> {
        CommandRunner::run("uv", &["sync"], &ctx.project_root)
    }

    fn add_dep(&self, ctx: &ProjectContext, dep: &str, dev: bool) -> Result<()> {
        let mut args = vec!["add", dep];
        if dev {
            args.push("--dev");
        }
        CommandRunner::run("uv", &args, &ctx.project_root)
    }

    fn remove_dep(&self, ctx: &ProjectContext, dep: &str) -> Result<()> {
        CommandRunner::run("uv", &["remove", dep], &ctx.project_root)
    }

    fn build(&self, ctx: &ProjectContext) -> Result<()> {
        CommandRunner::run_with_flags("uv", &["build"], &ctx.build_flags(), &ctx.project_root)
    }

    fn run(&self, ctx: &ProjectContext) -> Result<()> {
        let module_name = ctx.project.name.replace('-', "_");
        CommandRunner::run_with_flags(
            "uv",
            &["run", "python", "-m", &module_name],
            &ctx.run_flags(),
            &ctx.project_root,
        )
    }

    fn test(&self, ctx: &ProjectContext) -> Result<()> {
        CommandRunner::run_with_flags(
            "uv",
            &["run", "pytest"],
            &ctx.test_flags(),
            &ctx.project_root,
        )
    }

    fn fmt(&self, ctx: &ProjectContext) -> Result<()> {
        CommandRunner::run("uv", &["run", "ruff", "format", "."], &ctx.project_root)
    }

    fn lint(&self, ctx: &ProjectContext) -> Result<()> {
        CommandRunner::run("uv", &["run", "ruff", "check", "."], &ctx.project_root)
    }

    fn clean(&self, ctx: &ProjectContext) -> Result<()> {
        for dir_name in ["dist", "__pycache__", ".ruff_cache"] {
            let path = ctx.project_root.join(dir_name);
            if path.exists() {
                std::fs::remove_dir_all(&path)?;
            }
        }
        Ok(())
    }

    fn ensure_toolchain(&self, ctx: &ProjectContext) -> Result<()> {
        if !tool_exists("uv") {
            eprintln!("  installing uv v{} to .fire/bin/...", UV_VERSION);
            let bin_dir = ctx.bin_dir();
            std::fs::create_dir_all(&bin_dir)?;
            let script = format!(
                "curl -LsSf https://astral.sh/uv/{}/install.sh | INSTALLER_NO_MODIFY_PATH=1 UV_INSTALL_DIR='{}' sh",
                UV_VERSION,
                bin_dir.display()
            );
            run_shell(&script, &ctx.workspace_root)?;
        }

        if let Some(version) = &ctx.project.toolchain.version {
            CommandRunner::run("uv", &["python", "install", version], &ctx.workspace_root)?;
            // Pin the version inside the project directory (only if it exists)
            if ctx.project_root.exists() {
                CommandRunner::run("uv", &["python", "pin", version], &ctx.project_root)?;
            }
        }
        Ok(())
    }
}
