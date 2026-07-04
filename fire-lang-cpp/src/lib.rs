use std::path::Path;

use fire_core::{FireError, LanguageBackend, ProjectContext, Result};
use fire_runner::platform;
use fire_runner::{
    CommandRunner, download_and_extract, download_file, make_executable, run_shell, tool_exists,
};

/// Pinned tool versions — bump these explicitly to upgrade.
const CMAKE_VERSION: &str = "3.31.6";
const NINJA_VERSION: &str = "1.12.1";
const VCPKG_TAG: &str = "2025.04.09";

// ---------------------------------------------------------------------------
// CMakeBackend — shared backend for both C and C++
// ---------------------------------------------------------------------------

/// A CMake-based backend parameterised over C vs C++.
pub struct CMakeBackend {
    is_cpp: bool,
}

impl CMakeBackend {
    pub fn cpp() -> Self {
        Self { is_cpp: true }
    }

    pub fn c() -> Self {
        Self { is_cpp: false }
    }

    fn default_standard(&self) -> &str {
        if self.is_cpp { "20" } else { "17" }
    }

    fn cmake_lang(&self) -> &str {
        if self.is_cpp { "CXX" } else { "C" }
    }

    fn main_filename(&self) -> &str {
        if self.is_cpp { "main.cpp" } else { "main.c" }
    }

    fn compiler_check(&self) -> (&str, &str) {
        if self.is_cpp {
            ("clang++", "g++")
        } else {
            ("clang", "gcc")
        }
    }
}

impl LanguageBackend for CMakeBackend {
    fn name(&self) -> &str {
        if self.is_cpp { "cpp" } else { "c" }
    }

    fn detect(&self, path: &Path) -> bool {
        path.join("CMakeLists.txt").exists()
    }

    fn init(&self, ctx: &ProjectContext) -> Result<()> {
        let src_dir = ctx.project_root.join("src");
        std::fs::create_dir_all(&src_dir)?;

        let standard = ctx
            .project
            .toolchain
            .version
            .as_deref()
            .unwrap_or(self.default_standard());

        let std_var = if self.is_cpp {
            "CMAKE_CXX_STANDARD"
        } else {
            "CMAKE_C_STANDARD"
        };

        // CMakeLists.txt
        let cmake = format!(
            r#"cmake_minimum_required(VERSION 3.21)
project({name} LANGUAGES {lang})

set({std_var} {standard})
set({std_var}_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

add_executable(${{PROJECT_NAME}} src/{main_file})
"#,
            name = ctx.project.name,
            lang = self.cmake_lang(),
            std_var = std_var,
            standard = standard,
            main_file = self.main_filename(),
        );
        std::fs::write(ctx.project_root.join("CMakeLists.txt"), cmake)?;

        // CMakePresets.json — uses Ninja + vcpkg from .fire/
        let vcpkg_toolchain = ctx
            .fire_dir()
            .join("vcpkg")
            .join("scripts")
            .join("buildsystems")
            .join("vcpkg.cmake");
        let presets = serde_json::json!({
            "version": 3,
            "configurePresets": [{
                "name": "default",
                "generator": "Ninja",
                "binaryDir": "${sourceDir}/build",
                "cacheVariables": {
                    "CMAKE_TOOLCHAIN_FILE": vcpkg_toolchain.to_string_lossy(),
                    "CMAKE_EXPORT_COMPILE_COMMANDS": "ON"
                }
            }],
            "buildPresets": [{"name": "default", "configurePreset": "default"}],
            "testPresets": [{"name": "default", "configurePreset": "default"}]
        });
        std::fs::write(
            ctx.project_root.join("CMakePresets.json"),
            serde_json::to_string_pretty(&presets).unwrap(),
        )?;

        // vcpkg.json manifest
        let vcpkg_manifest = serde_json::json!({
            "name": ctx.project.name,
            "version-string": "0.1.0",
            "dependencies": []
        });
        std::fs::write(
            ctx.project_root.join("vcpkg.json"),
            serde_json::to_string_pretty(&vcpkg_manifest).unwrap(),
        )?;

        // Source file
        if self.is_cpp {
            std::fs::write(
                src_dir.join("main.cpp"),
                format!(
                    "#include <iostream>\n\nint main() {{\n    std::cout << \"Hello from {}!\" << std::endl;\n    return 0;\n}}\n",
                    ctx.project.name
                ),
            )?;
        } else {
            std::fs::write(
                src_dir.join("main.c"),
                format!(
                    "#include <stdio.h>\n\nint main(void) {{\n    printf(\"Hello from {}!\\n\");\n    return 0;\n}}\n",
                    ctx.project.name
                ),
            )?;
        }

        // .clangd config
        std::fs::write(
            ctx.project_root.join(".clangd"),
            "CompileFlags:\n  CompilationDatabase: build/\n",
        )?;

        Ok(())
    }

    fn install(&self, ctx: &ProjectContext) -> Result<()> {
        self.configure(ctx)
    }

    fn add_dep(&self, ctx: &ProjectContext, dep: &str, _dev: bool) -> Result<()> {
        let manifest_path = ctx.project_root.join("vcpkg.json");
        let content = std::fs::read_to_string(&manifest_path)?;
        let mut manifest: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| FireError::Config(format!("invalid vcpkg.json: {}", e)))?;

        if let Some(deps) = manifest
            .get_mut("dependencies")
            .and_then(|d| d.as_array_mut())
        {
            let dep_val = serde_json::Value::String(dep.to_string());
            if !deps.contains(&dep_val) {
                deps.push(dep_val);
            }
        }

        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap(),
        )?;
        Ok(())
    }

    fn remove_dep(&self, ctx: &ProjectContext, dep: &str) -> Result<()> {
        let manifest_path = ctx.project_root.join("vcpkg.json");
        let content = std::fs::read_to_string(&manifest_path)?;
        let mut manifest: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| FireError::Config(format!("invalid vcpkg.json: {}", e)))?;

        if let Some(deps) = manifest
            .get_mut("dependencies")
            .and_then(|d| d.as_array_mut())
        {
            deps.retain(|d| d.as_str() != Some(dep));
        }

        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap(),
        )?;
        Ok(())
    }

    fn build(&self, ctx: &ProjectContext) -> Result<()> {
        self.configure(ctx)?;
        CommandRunner::run_with_flags(
            "cmake",
            &["--build", "build"],
            &ctx.build_flags(),
            &ctx.project_root,
        )
    }

    fn run(&self, ctx: &ProjectContext) -> Result<()> {
        let binary = ctx.project_root.join("build").join(&ctx.project.name);
        if !binary.exists() {
            self.build(ctx)?;
        }
        let bin_str = binary.to_string_lossy();
        CommandRunner::run_with_flags(&bin_str, &[], &ctx.run_flags(), &ctx.project_root)
    }

    fn test(&self, ctx: &ProjectContext) -> Result<()> {
        self.configure(ctx)?;
        CommandRunner::run("cmake", &["--build", "build"], &ctx.project_root)?;
        CommandRunner::run_with_flags(
            "ctest",
            &["--test-dir", "build"],
            &ctx.test_flags(),
            &ctx.project_root,
        )
    }

    fn fmt(&self, ctx: &ProjectContext) -> Result<()> {
        let src_dir = ctx.project_root.join("src");
        let files = collect_c_source_files(&src_dir)?;
        if files.is_empty() {
            return Ok(());
        }
        let mut args: Vec<&str> = vec!["-i"];
        let file_strs: Vec<String> = files
            .iter()
            .map(|f| f.to_string_lossy().to_string())
            .collect();
        let file_refs: Vec<&str> = file_strs.iter().map(|s| s.as_str()).collect();
        args.extend(&file_refs);
        CommandRunner::run("clang-format", &args, &ctx.project_root)
    }

    fn lint(&self, ctx: &ProjectContext) -> Result<()> {
        let src_dir = ctx.project_root.join("src");
        let files = collect_c_source_files(&src_dir)?;
        if files.is_empty() {
            return Ok(());
        }
        let mut args: Vec<&str> = vec!["-p", "build"];
        let file_strs: Vec<String> = files
            .iter()
            .map(|f| f.to_string_lossy().to_string())
            .collect();
        let file_refs: Vec<&str> = file_strs.iter().map(|s| s.as_str()).collect();
        args.extend(&file_refs);
        CommandRunner::run("clang-tidy", &args, &ctx.project_root)
    }

    fn clean(&self, ctx: &ProjectContext) -> Result<()> {
        let build_dir = ctx.project_root.join("build");
        if build_dir.exists() {
            std::fs::remove_dir_all(&build_dir)?;
        }
        Ok(())
    }

    fn ensure_toolchain(&self, ctx: &ProjectContext) -> Result<()> {
        let (primary, fallback) = self.compiler_check();
        if !tool_exists(primary) && !tool_exists(fallback) {
            return Err(FireError::ToolchainNotFound {
                tool: format!(
                    "{} or {} (install via: apt install clang or apt install gcc/g++)",
                    primary, fallback
                ),
            });
        }

        if !tool_exists("cmake") {
            eprintln!("  installing cmake v{} to .fire/cmake/...", CMAKE_VERSION);
            self.install_cmake(ctx)?;
        }

        if !tool_exists("ninja") {
            eprintln!("  installing ninja v{} to .fire/bin/...", NINJA_VERSION);
            self.install_ninja(ctx)?;
        }

        let vcpkg_dir = ctx.fire_dir().join("vcpkg");
        if !vcpkg_dir.join("vcpkg").exists() {
            eprintln!("  installing vcpkg ({}) to .fire/vcpkg/...", VCPKG_TAG);
            self.install_vcpkg(ctx)?;
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Private install helpers
// ---------------------------------------------------------------------------

impl CMakeBackend {
    fn configure(&self, ctx: &ProjectContext) -> Result<()> {
        CommandRunner::run("cmake", &["--preset", "default"], &ctx.project_root)
    }

    fn install_cmake(&self, ctx: &ProjectContext) -> Result<()> {
        let cmake_dir = ctx.fire_dir().join("cmake");
        std::fs::create_dir_all(&cmake_dir)?;
        let url = format!(
            "https://github.com/Kitware/CMake/releases/download/v{}/cmake-{}-{}.{}",
            CMAKE_VERSION,
            CMAKE_VERSION,
            platform::cmake_platform(),
            platform::cmake_archive_ext()
        );
        download_and_extract(&url, &cmake_dir, 1)?;
        Ok(())
    }

    fn install_ninja(&self, ctx: &ProjectContext) -> Result<()> {
        let bin_dir = ctx.bin_dir();
        std::fs::create_dir_all(&bin_dir)?;
        let archive_name = platform::ninja_archive_name();
        let zip_path = bin_dir.join(archive_name);
        let url = format!(
            "https://github.com/ninja-build/ninja/releases/download/v{}/{}",
            NINJA_VERSION, archive_name
        );
        download_file(&url, &zip_path)?;
        run_shell(
            &format!(
                "unzip -o '{}' -d '{}'",
                zip_path.display(),
                bin_dir.display()
            ),
            &ctx.workspace_root,
        )?;
        make_executable(&bin_dir.join(format!("ninja{}", platform::exe_suffix())))?;
        let _ = std::fs::remove_file(&zip_path);
        Ok(())
    }

    fn install_vcpkg(&self, ctx: &ProjectContext) -> Result<()> {
        let vcpkg_dir = ctx.fire_dir().join("vcpkg");
        let vcpkg_dir_str = vcpkg_dir.to_string_lossy().to_string();
        CommandRunner::run(
            "git",
            &[
                "clone",
                "--depth",
                "1",
                "--branch",
                VCPKG_TAG,
                "https://github.com/microsoft/vcpkg.git",
                &vcpkg_dir_str,
            ],
            &ctx.workspace_root,
        )?;
        let bootstrap = vcpkg_dir.join("bootstrap-vcpkg.sh");
        run_shell(
            &format!("'{}' -disableMetrics", bootstrap.display()),
            &vcpkg_dir,
        )?;
        Ok(())
    }
}

/// Collect C/C++ source and header files from a directory recursively.
fn collect_c_source_files(dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut files = Vec::new();
    if !dir.exists() {
        return Ok(files);
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            files.extend(collect_c_source_files(&path)?);
        } else if let Some("c" | "cpp" | "cc" | "cxx" | "h" | "hpp" | "hxx") =
            path.extension().and_then(|e| e.to_str())
        {
            files.push(path);
        }
    }
    Ok(files)
}
