use std::path::Path;

use fire_core::{LanguageBackend, Result, ProjectContext};
use fire_runner::{download_and_extract, tool_exists, CommandRunner};
use fire_runner::platform;

/// Pinned tool versions — bump these explicitly to upgrade.
const NODE_VERSION: &str = "22.16.0";
const PNPM_VERSION: &str = "11.9.0";

pub struct TypeScriptBackend;

impl TypeScriptBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TypeScriptBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageBackend for TypeScriptBackend {
    fn name(&self) -> &str {
        "typescript"
    }

    fn detect(&self, path: &Path) -> bool {
        path.join("package.json").exists()
    }

    fn init(&self, ctx: &ProjectContext) -> Result<()> {
        let src_dir = ctx.project_root.join("src");
        std::fs::create_dir_all(&src_dir)?;

        // Scaffold package.json with sensible scripts
        let package_json = format!(
            r#"{{
  "name": "{name}",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "scripts": {{
    "start": "tsx src/index.ts",
    "build": "tsc",
    "test": "echo \"no tests configured\" && exit 0",
    "lint": "echo \"no linter configured\" && exit 0",
    "format": "echo \"no formatter configured\" && exit 0"
  }}
}}"#,
            name = ctx.project.name
        );
        std::fs::write(ctx.project_root.join("package.json"), package_json)?;

        // pnpm v11 requires explicit build approval; create pnpm-workspace.yaml
        std::fs::write(
            ctx.project_root.join("pnpm-workspace.yaml"),
            "allowBuilds:\n  esbuild: true\n",
        )?;

        // Scaffold tsconfig.json
        let tsconfig = r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "Node16",
    "moduleResolution": "Node16",
    "outDir": "dist",
    "rootDir": "src",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "forceConsistentCasingInFileNames": true,
    "declaration": true
  },
  "include": ["src"],
  "exclude": ["node_modules", "dist"]
}"#;
        std::fs::write(ctx.project_root.join("tsconfig.json"), tsconfig)?;

        // Scaffold src/index.ts
        std::fs::write(
            src_dir.join("index.ts"),
            format!("console.log(\"Hello from {}!\");\n", ctx.project.name),
        )?;

        // Install dev dependencies
        CommandRunner::run(
            "pnpm",
            &["add", "-D", "typescript", "tsx", "@types/node"],
            &ctx.project_root,
        )?;

        Ok(())
    }

    fn install(&self, ctx: &ProjectContext) -> Result<()> {
        CommandRunner::run("pnpm", &["install"], &ctx.project_root)
    }

    fn add_dep(&self, ctx: &ProjectContext, dep: &str, dev: bool) -> Result<()> {
        let mut args = vec!["add"];
        if dev {
            args.push("-D");
        }
        args.push(dep);
        CommandRunner::run("pnpm", &args, &ctx.project_root)
    }

    fn remove_dep(&self, ctx: &ProjectContext, dep: &str) -> Result<()> {
        CommandRunner::run("pnpm", &["remove", dep], &ctx.project_root)
    }

    fn build(&self, ctx: &ProjectContext) -> Result<()> {
        CommandRunner::run_with_flags(
            "pnpm",
            &["run", "build"],
            &ctx.build_flags(),
            &ctx.project_root,
        )
    }

    fn run(&self, ctx: &ProjectContext) -> Result<()> {
        CommandRunner::run_with_flags(
            "pnpm",
            &["run", "start"],
            &ctx.run_flags(),
            &ctx.project_root,
        )
    }

    fn test(&self, ctx: &ProjectContext) -> Result<()> {
        CommandRunner::run_with_flags(
            "pnpm",
            &["run", "test"],
            &ctx.test_flags(),
            &ctx.project_root,
        )
    }

    fn fmt(&self, ctx: &ProjectContext) -> Result<()> {
        CommandRunner::run("pnpm", &["run", "format"], &ctx.project_root)
    }

    fn lint(&self, ctx: &ProjectContext) -> Result<()> {
        CommandRunner::run("pnpm", &["run", "lint"], &ctx.project_root)
    }

    fn clean(&self, ctx: &ProjectContext) -> Result<()> {
        let dist = ctx.project_root.join("dist");
        if dist.exists() {
            std::fs::remove_dir_all(&dist)?;
        }
        Ok(())
    }

    fn ensure_toolchain(&self, ctx: &ProjectContext) -> Result<()> {
        // 1. Ensure Node.js is available
        if !tool_exists("node") {
            let version = ctx
                .project
                .toolchain
                .version
                .as_deref()
                .unwrap_or(NODE_VERSION);
            eprintln!("  installing node v{} to .fire/node/...", version);
            self.install_node(ctx, version)?;
        } else if let Some(version) = &ctx.project.toolchain.version {
            // Check version match
            if let Ok(output) =
                CommandRunner::run_capture("node", &["--version"], &ctx.workspace_root)
            {
                let current = output.trim().trim_start_matches('v');
                if !current.starts_with(version.as_str()) {
                    eprintln!(
                        "  warning: Node.js v{} found but v{}.x requested",
                        current, version
                    );
                }
            }
        }

        // 2. Ensure pnpm is available
        if !tool_exists("pnpm") {
            eprintln!("  installing pnpm to .fire/bin/...");
            self.install_pnpm(ctx)?;
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Private install helpers
// ---------------------------------------------------------------------------

impl TypeScriptBackend {
    fn install_node(&self, ctx: &ProjectContext, version: &str) -> Result<()> {
        let node_dir = ctx.fire_dir().join("node");
        std::fs::create_dir_all(&node_dir)?;

        let url = format!(
            "https://nodejs.org/dist/v{}/node-v{}-{}.{}",
            version, version, platform::node_platform(), platform::node_archive_ext()
        );
        download_and_extract(&url, &node_dir, 1)?;
        Ok(())
    }

    fn install_pnpm(&self, ctx: &ProjectContext) -> Result<()> {
        // Use npm (available from the node installation) to install pnpm
        // into .fire/bin/ via --prefix. This avoids the pnpm install script's
        // side effects (e.g. modifying .bashrc).
        let fire_dir = ctx.fire_dir();
        let fire_dir_str = fire_dir.to_string_lossy();
        let pnpm_spec = format!("pnpm@{}", PNPM_VERSION);
        CommandRunner::run(
            "npm",
            &["install", "-g", "--prefix", &fire_dir_str, &pnpm_spec],
            &ctx.workspace_root,
        )
    }
}
