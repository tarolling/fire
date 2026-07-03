use std::path::Path;

use fire_core::{LanguageBackend, Result, ProjectContext};
use fire_runner::{run_shell, tool_exists, CommandRunner};

pub struct RustBackend;

impl RustBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RustBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageBackend for RustBackend {
    fn name(&self) -> &str {
        "rust"
    }

    fn detect(&self, path: &Path) -> bool {
        path.join("Cargo.toml").exists()
    }

    fn init(&self, ctx: &ProjectContext) -> Result<()> {
        std::fs::create_dir_all(&ctx.project_root)?;
        CommandRunner::run(
            "cargo",
            &["init", "--name", &ctx.project.name],
            &ctx.project_root,
        )
    }

    fn install(&self, ctx: &ProjectContext) -> Result<()> {
        CommandRunner::run("cargo", &["fetch"], &ctx.project_root)
    }

    fn add_dep(&self, ctx: &ProjectContext, dep: &str, dev: bool) -> Result<()> {
        let mut args = vec!["add", dep];
        if dev {
            args.push("--dev");
        }
        CommandRunner::run("cargo", &args, &ctx.project_root)
    }

    fn remove_dep(&self, ctx: &ProjectContext, dep: &str) -> Result<()> {
        CommandRunner::run("cargo", &["remove", dep], &ctx.project_root)
    }

    fn build(&self, ctx: &ProjectContext) -> Result<()> {
        CommandRunner::run_with_flags("cargo", &["build"], &ctx.build_flags(), &ctx.project_root)
    }

    fn run(&self, ctx: &ProjectContext) -> Result<()> {
        CommandRunner::run_with_flags("cargo", &["run"], &ctx.run_flags(), &ctx.project_root)
    }

    fn test(&self, ctx: &ProjectContext) -> Result<()> {
        CommandRunner::run_with_flags("cargo", &["test"], &ctx.test_flags(), &ctx.project_root)
    }

    fn fmt(&self, ctx: &ProjectContext) -> Result<()> {
        CommandRunner::run("cargo", &["fmt"], &ctx.project_root)
    }

    fn lint(&self, ctx: &ProjectContext) -> Result<()> {
        CommandRunner::run("cargo", &["clippy", "--", "-D", "warnings"], &ctx.project_root)
    }

    fn clean(&self, ctx: &ProjectContext) -> Result<()> {
        CommandRunner::run("cargo", &["clean"], &ctx.project_root)
    }

    fn ensure_toolchain(&self, ctx: &ProjectContext) -> Result<()> {
        if !tool_exists("cargo") {
            eprintln!("  installing rust toolchain to .fire/...");
            let fire_dir = ctx.fire_dir();
            // Point rustup/cargo at project-local directories before installing
            // SAFETY: called from single-threaded CLI dispatch, before any
            //         parallel work.
            unsafe {
                std::env::set_var("RUSTUP_HOME", fire_dir.join("rustup"));
                std::env::set_var("CARGO_HOME", fire_dir.join("cargo"));
            }
            run_shell(
                "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path",
                &ctx.workspace_root,
            )?;
        }

        if let Some(version) = &ctx.project.toolchain.version {
            CommandRunner::run(
                "rustup",
                &["toolchain", "install", version],
                &ctx.workspace_root,
            )?;
            if ctx.project_root.exists() {
                CommandRunner::run(
                    "rustup",
                    &["override", "set", version],
                    &ctx.project_root,
                )?;
            }
        }
        Ok(())
    }
}
