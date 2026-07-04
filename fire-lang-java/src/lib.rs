use std::path::Path;

use fire_core::{LanguageBackend, ProjectContext, Result};
use fire_runner::platform;
use fire_runner::{CommandRunner, download_and_extract, run_shell, tool_exists};

/// Pinned tool versions — bump these explicitly to upgrade.
const JDK_VERSION: &str = "21.0.7+6";
const GRADLE_VERSION: &str = "8.12";

pub struct JavaBackend;

impl JavaBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for JavaBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageBackend for JavaBackend {
    fn name(&self) -> &str {
        "java"
    }

    fn detect(&self, path: &Path) -> bool {
        path.join("build.gradle.kts").exists() || path.join("build.gradle").exists()
    }

    fn init(&self, ctx: &ProjectContext) -> Result<()> {
        std::fs::create_dir_all(&ctx.project_root)?;

        let java_version = ctx.project.toolchain.version.as_deref().unwrap_or("21");

        // Use Gradle to generate a Java application project.
        // Providing all args avoids interactive prompts.
        CommandRunner::run(
            "gradle",
            &[
                "init",
                "--type",
                "java-application",
                "--dsl",
                "kotlin",
                "--java-version",
                java_version,
                "--project-name",
                &ctx.project.name,
                "--package",
                &format!("com.{}", ctx.project.name.replace('-', "")),
            ],
            &ctx.project_root,
        )?;

        // Remove the foojay-resolver plugin from settings.gradle.kts — it
        // requires network access and is unnecessary since fire manages the JDK.
        let settings_file = ctx.project_root.join("settings.gradle.kts");
        if settings_file.exists() {
            let content = std::fs::read_to_string(&settings_file)?;
            let filtered: String = content
                .lines()
                .filter(|line| !line.contains("foojay"))
                .collect::<Vec<_>>()
                .join("\n");
            std::fs::write(&settings_file, format!("{}\n", filtered))?;
        }

        // Remove the java.toolchain block from build.gradle.kts — it requires
        // a toolchain resolver plugin to auto-download JDKs. Since fire manages
        // the JDK, we just set source/target compatibility directly.
        // Also strip the guava dependency so the project runs without network.
        let build_file = ctx.project_root.join("app").join("build.gradle.kts");
        if build_file.exists() {
            let content = std::fs::read_to_string(&build_file)?;
            let mut result = String::new();
            let mut skip_depth = 0i32;
            let mut in_java_block = false;
            for line in content.lines() {
                if line.trim().starts_with("java {") || line.trim() == "java{" {
                    in_java_block = true;
                    skip_depth = 1;
                    continue;
                }
                if in_java_block {
                    skip_depth += line.matches('{').count() as i32;
                    skip_depth -= line.matches('}').count() as i32;
                    if skip_depth <= 0 {
                        in_java_block = false;
                    }
                    continue;
                }
                // Strip guava references (added by gradle init but requires network)
                if line.contains("libs.guava") || line.contains("guava") {
                    continue;
                }
                result.push_str(line);
                result.push('\n');
            }
            std::fs::write(&build_file, result)?;
        }

        // Also clean up the version catalog
        let catalog = ctx.project_root.join("gradle").join("libs.versions.toml");
        if catalog.exists() {
            let content = std::fs::read_to_string(&catalog)?;
            let filtered: String = content
                .lines()
                .filter(|line| !line.contains("guava"))
                .collect::<Vec<_>>()
                .join("\n");
            std::fs::write(&catalog, format!("{}\n", filtered))?;
        }

        Ok(())
    }

    fn install(&self, ctx: &ProjectContext) -> Result<()> {
        self.gradlew(ctx, &["dependencies"])
    }

    fn add_dep(&self, ctx: &ProjectContext, dep: &str, dev: bool) -> Result<()> {
        let build_file = ctx.project_root.join("app").join("build.gradle.kts");
        let content = std::fs::read_to_string(&build_file)?;

        let config = if dev {
            "testImplementation"
        } else {
            "implementation"
        };
        let dep_line = format!("    {}(\"{}\")", config, dep);

        // Insert before the closing brace of the dependencies block
        let new_content = if let Some(pos) = content.rfind("\n}\n") {
            // Check if we're in a dependencies block by looking backwards
            format!("{}{}\n{}", &content[..pos], &dep_line, &content[pos..])
        } else {
            // Append a dependencies block
            format!(
                "{}\ndependencies {{\n{}\n}}\n",
                content.trim_end(),
                dep_line
            )
        };

        std::fs::write(&build_file, new_content)?;
        Ok(())
    }

    fn remove_dep(&self, ctx: &ProjectContext, dep: &str) -> Result<()> {
        let build_file = ctx.project_root.join("app").join("build.gradle.kts");
        let content = std::fs::read_to_string(&build_file)?;

        let new_content: String = content
            .lines()
            .filter(|line| !line.contains(&format!("\"{}\"", dep)))
            .collect::<Vec<_>>()
            .join("\n");

        std::fs::write(&build_file, format!("{}\n", new_content))?;
        Ok(())
    }

    fn build(&self, ctx: &ProjectContext) -> Result<()> {
        self.gradlew_with_flags(ctx, &["build", "-x", "test"], &ctx.build_flags())
    }

    fn run(&self, ctx: &ProjectContext) -> Result<()> {
        self.gradlew_with_flags(ctx, &["run"], &ctx.run_flags())
    }

    fn test(&self, ctx: &ProjectContext) -> Result<()> {
        self.gradlew_with_flags(ctx, &["test"], &ctx.test_flags())
    }

    fn fmt(&self, ctx: &ProjectContext) -> Result<()> {
        // google-java-format via Gradle spotless plugin if configured,
        // otherwise skip silently
        let _ = self.gradlew(ctx, &["spotlessApply"]);
        Ok(())
    }

    fn lint(&self, ctx: &ProjectContext) -> Result<()> {
        self.gradlew(ctx, &["check"])
    }

    fn clean(&self, ctx: &ProjectContext) -> Result<()> {
        self.gradlew(ctx, &["clean"])
    }

    fn ensure_toolchain(&self, ctx: &ProjectContext) -> Result<()> {
        // 1. Ensure the fire-managed JDK is installed (always use ours to
        //    guarantee compatibility with our pinned Gradle version).
        let jdk_dir = ctx.fire_dir().join("jdk");
        if !jdk_dir.join("bin").join("java").exists() {
            eprintln!("  installing jdk {} to .fire/jdk/...", JDK_VERSION);
            self.install_jdk(ctx)?;
        }

        // Set JAVA_HOME so Gradle uses our JDK (already first in PATH via setup_tool_env)
        // SAFETY: called from single-threaded CLI dispatch.
        unsafe { std::env::set_var("JAVA_HOME", &jdk_dir) };

        // 2. Ensure Gradle is available (for initial project scaffolding)
        if !tool_exists("gradle") {
            eprintln!(
                "  installing gradle v{} to .fire/gradle/...",
                GRADLE_VERSION
            );
            self.install_gradle(ctx)?;
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Private install helpers
// ---------------------------------------------------------------------------

impl JavaBackend {
    fn gradlew(&self, ctx: &ProjectContext, args: &[&str]) -> Result<()> {
        // Prefer fire-managed gradle directly (avoids wrapper re-downloading)
        if tool_exists("gradle") {
            CommandRunner::run("gradle", args, &ctx.project_root)
        } else {
            let gradlew = ctx.project_root.join("gradlew");
            let gradlew_str = gradlew.to_string_lossy();
            CommandRunner::run(&gradlew_str, args, &ctx.project_root)
        }
    }

    fn gradlew_with_flags(
        &self,
        ctx: &ProjectContext,
        args: &[&str],
        extra_flags: &[String],
    ) -> Result<()> {
        if tool_exists("gradle") {
            CommandRunner::run_with_flags("gradle", args, extra_flags, &ctx.project_root)
        } else {
            let gradlew = ctx.project_root.join("gradlew");
            let gradlew_str = gradlew.to_string_lossy();
            CommandRunner::run_with_flags(&gradlew_str, args, extra_flags, &ctx.project_root)
        }
    }

    fn install_jdk(&self, ctx: &ProjectContext) -> Result<()> {
        let jdk_dir = ctx.fire_dir().join("jdk");
        std::fs::create_dir_all(&jdk_dir)?;

        // Adoptium Temurin JDK — pinned version
        // Version format: 21.0.7+6 → URL uses 21.0.7%2B6 and filename uses 21.0.7_6
        let version_url = JDK_VERSION.replace('+', "%2B");
        let version_file = JDK_VERSION.replace('+', "_");
        let major = JDK_VERSION.split('.').next().unwrap_or("21");

        let url = format!(
            "https://github.com/adoptium/temurin{}-binaries/releases/download/jdk-{}/OpenJDK{}U-jdk_{}_{}_hotspot_{}.{}",
            major,
            version_url,
            major,
            platform::jdk_arch(),
            platform::jdk_os(),
            version_file,
            platform::jdk_archive_ext()
        );
        download_and_extract(&url, &jdk_dir, 1)?;
        Ok(())
    }

    fn install_gradle(&self, ctx: &ProjectContext) -> Result<()> {
        let gradle_dir = ctx.fire_dir().join("gradle");
        std::fs::create_dir_all(&gradle_dir)?;

        let url = format!(
            "https://services.gradle.org/distributions/gradle-{}-bin.zip",
            GRADLE_VERSION
        );
        let zip_path = gradle_dir.join("gradle.zip");

        CommandRunner::run(
            "curl",
            &["-fsSL", "-o", &zip_path.to_string_lossy(), &url],
            &ctx.workspace_root,
        )?;
        run_shell(
            &format!(
                "unzip -o '{}' -d '{}' && mv '{}/gradle-{}'/* '{}'",
                zip_path.display(),
                gradle_dir.display(),
                gradle_dir.display(),
                GRADLE_VERSION,
                gradle_dir.display(),
            ),
            &ctx.workspace_root,
        )?;
        // Keep the zip for gradle-wrapper to reference locally
        let nested = gradle_dir.join(format!("gradle-{}", GRADLE_VERSION));
        let _ = std::fs::remove_dir_all(&nested);

        Ok(())
    }
}
