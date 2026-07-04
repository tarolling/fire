use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use owo_colors::OwoColorize;

use fire_config::{WorkspaceConfig, config_path, discover_workspace};
use fire_core::{
    BackendRegistry, FireError, Language, LanguageBackend, ProjectConfig, ProjectContext,
    ToolchainConfig,
};
use fire_runner::CommandRunner;

mod ui;

/// Extra bash completion logic that adds dynamic project name completion.
/// Appended after clap_complete's generated script.
const BASH_DYNAMIC_COMPLETIONS: &str = r#"
# Dynamic project name completion for fire
_fire_projects() {
    fire complete-projects 2>/dev/null
}

# Wrap the generated _fire function to inject project names
_fire_dynamic() {
    _fire "$@"
    local cmd="${COMP_WORDS[1]}"
    case "$cmd" in
        build|run|test|fmt|lint|clean|install|exec|delete)
            if [[ ${#COMPREPLY[@]} -eq 0 || "${COMP_WORDS[COMP_CWORD]}" != -* ]]; then
                local projects=$(_fire_projects)
                COMPREPLY+=($(compgen -W "$projects" -- "${COMP_WORDS[COMP_CWORD]}"))
            fi
            ;;
        add|remove)
            if [[ "$prev" == "--project" || "$prev" == "-p" ]]; then
                local projects=$(_fire_projects)
                COMPREPLY=($(compgen -W "$projects" -- "${COMP_WORDS[COMP_CWORD]}"))
            fi
            ;;
    esac
}

complete -F _fire_dynamic -o default fire
"#;

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(
    name = "fire",
    version,
    about = "Universal project & workspace manager"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new fire workspace
    Init {
        /// Workspace name (defaults to current directory name)
        name: Option<String>,
        /// Version control system to initialize (git or none)
        #[arg(long, default_value = "git")]
        vcs: String,
    },

    /// Create a new project in the workspace
    New {
        /// Project name
        name: String,
        /// Programming language (rust, python, typescript, cpp, c)
        #[arg(short, long)]
        lang: String,
        /// Project path (defaults to projects/<name>)
        #[arg(short, long)]
        path: Option<String>,
    },

    /// Install dependencies for projects
    Install {
        /// Target projects (all if none specified)
        projects: Vec<String>,
    },

    /// Add a dependency to a project
    Add {
        /// Dependency specifier (e.g. serde, requests, express)
        dep: String,
        /// Target project
        #[arg(short, long)]
        project: String,
        /// Add as development dependency
        #[arg(long)]
        dev: bool,
    },

    /// Remove a dependency from a project
    Remove {
        /// Dependency name
        dep: String,
        /// Target project
        #[arg(short, long)]
        project: String,
    },

    /// Remove a project from the workspace and delete its files
    Delete {
        /// Project name to remove
        name: String,
        /// Skip confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },

    /// Build projects
    Build {
        /// Target projects (all if none specified)
        projects: Vec<String>,
        /// Extra flags passed to the build tool (after --)
        #[arg(last = true)]
        flags: Vec<String>,
    },

    /// Run projects
    Run {
        /// Target projects (all if none specified)
        projects: Vec<String>,
        /// Extra flags passed to the run tool (after --)
        #[arg(last = true)]
        flags: Vec<String>,
    },

    /// Test projects
    Test {
        /// Target projects (all if none specified)
        projects: Vec<String>,
        /// Extra flags passed to the test tool (after --)
        #[arg(last = true)]
        flags: Vec<String>,
    },

    /// Format code in projects
    Fmt {
        /// Target projects (all if none specified)
        projects: Vec<String>,
    },

    /// Lint projects
    Lint {
        /// Target projects (all if none specified)
        projects: Vec<String>,
    },

    /// Clean build artifacts
    Clean {
        /// Target projects (all if none specified)
        projects: Vec<String>,
    },

    /// Show workspace status
    Status,

    /// Execute an arbitrary command in a project's directory
    Exec {
        /// Target project
        project: String,
        /// Command and arguments (after --)
        #[arg(last = true)]
        cmd: Vec<String>,
    },

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        shell: Shell,
    },

    /// List project names (used internally for shell completions)
    #[command(hide = true)]
    CompleteProjects,
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

fn build_registry() -> BackendRegistry {
    let mut registry = BackendRegistry::new();
    registry.register(Language::Rust, Box::new(fire_lang_rust::RustBackend::new()));
    registry.register(
        Language::Python,
        Box::new(fire_lang_python::PythonBackend::new()),
    );
    registry.register(
        Language::TypeScript,
        Box::new(fire_lang_ts::TypeScriptBackend::new()),
    );
    registry.register(Language::Cpp, Box::new(fire_lang_cpp::CMakeBackend::cpp()));
    registry.register(Language::C, Box::new(fire_lang_cpp::CMakeBackend::c()));
    registry.register(Language::Java, Box::new(fire_lang_java::JavaBackend::new()));
    registry
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn resolve_projects(
    config: &WorkspaceConfig,
    names: &[String],
) -> fire_core::Result<Vec<ProjectConfig>> {
    if names.is_empty() {
        Ok(config.projects.clone())
    } else {
        names
            .iter()
            .map(|name| config.find_project(name).cloned())
            .collect()
    }
}

/// Discover workspace and configure the project-local tool environment.
fn load_workspace() -> Result<(std::path::PathBuf, WorkspaceConfig)> {
    let cwd = std::env::current_dir()?;
    let (workspace_root, config) = discover_workspace(&cwd)?;
    fire_core::setup_tool_env(&workspace_root);
    Ok((workspace_root, config))
}

/// Run an action across one or more projects, printing headers and a summary.
fn for_each_project<F>(
    action_name: &str,
    project_names: &[String],
    passthrough_flags: &[String],
    action: F,
) -> Result<()>
where
    F: Fn(&dyn LanguageBackend, &ProjectContext) -> fire_core::Result<()>,
{
    let (workspace_root, config) = load_workspace()?;
    let registry = build_registry();
    let projects = resolve_projects(&config, project_names)?;

    if projects.is_empty() {
        ui::warn("no projects defined in workspace — use 'fire new' to create one");
        return Ok(());
    }

    let total = projects.len();
    let mut succeeded = 0;

    for proj in &projects {
        ui::project_header(&proj.name, &proj.language.to_string());

        let backend = registry.get(&proj.language)?;
        let ctx = ProjectContext::new(proj.clone(), &workspace_root, passthrough_flags.to_vec());

        if let Err(e) = backend.ensure_toolchain(&ctx) {
            ui::error(&format!("toolchain: {}", e));
            continue;
        }

        match action(backend, &ctx) {
            Ok(()) => {
                succeeded += 1;
            }
            Err(e) => {
                ui::error(&format!("{}", e));
            }
        }
    }

    ui::summary(succeeded, total, action_name);

    if succeeded < total {
        std::process::exit(1);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn cmd_init(name: Option<String>, vcs: &str) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let cfg = config_path(&cwd);

    if cfg.exists() {
        anyhow::bail!("fire.toml already exists in {}", cwd.display());
    }

    let workspace_name = name.unwrap_or_else(|| {
        cwd.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "my-project".to_string())
    });

    let config = WorkspaceConfig::new(&workspace_name);
    config.save(&cfg)?;

    // Initialize VCS
    match vcs {
        "git" => {
            if !cwd.join(".git").exists() {
                let _ = CommandRunner::run("git", &["init"], &cwd);
            }
        }
        "none" => {}
        other => anyhow::bail!("unsupported vcs: '{}' (use 'git' or 'none')", other),
    }

    // Ensure .fire/ is git-ignored
    let gitignore = cwd.join(".gitignore");
    if gitignore.exists() {
        let content = std::fs::read_to_string(&gitignore)?;
        if !content.contains(".fire") {
            std::fs::write(&gitignore, format!("{}\n.fire/\n", content.trim_end()))?;
        }
    } else if vcs == "git" {
        std::fs::write(&gitignore, ".fire/\n")?;
    }

    ui::success(&format!("initialized workspace '{}'", workspace_name));
    Ok(())
}

fn cmd_new(name: String, lang: String, path: Option<String>) -> Result<()> {
    let (workspace_root, mut config) = load_workspace()?;
    let registry = build_registry();

    let language: Language = lang
        .parse()
        .map_err(|_| FireError::UnsupportedLanguage(lang.clone()))?;

    let proj_path = path.unwrap_or_else(|| format!("projects/{}", name));

    let proj_config = ProjectConfig {
        name: name.clone(),
        path: proj_path,
        language,
        toolchain: ToolchainConfig::default(),
    };

    let backend = registry.get(&language)?;
    let ctx = ProjectContext::new(proj_config.clone(), &workspace_root, vec![]);

    // Create directory, verify toolchain, then scaffold
    std::fs::create_dir_all(&ctx.project_root)?;
    backend.ensure_toolchain(&ctx)?;
    backend.init(&ctx)?;

    config.add_project(proj_config)?;
    config.save(&config_path(&workspace_root))?;

    ui::success(&format!("created project '{}' ({})", name, language));
    Ok(())
}

fn cmd_status() -> Result<()> {
    let (workspace_root, config) = load_workspace()?;

    println!("{} {}", "workspace:".bold(), config.workspace.name);
    println!("{} {}", "     root:".bold(), workspace_root.display());
    println!();

    if config.projects.is_empty() {
        println!("  no projects defined — use 'fire new' to create one");
        return Ok(());
    }

    for proj in &config.projects {
        let proj_path = workspace_root.join(&proj.path);
        let exists = proj_path.exists();
        let marker = if exists {
            "✓".green().to_string()
        } else {
            "✗".red().to_string()
        };

        let version_str = proj
            .toolchain
            .version
            .as_deref()
            .map(|v| format!(" [v{}]", v))
            .unwrap_or_default();

        println!(
            "  {} {} ({}){} → {}",
            marker,
            proj.name.bold(),
            proj.language,
            version_str,
            proj.path,
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { name, vcs } => cmd_init(name, &vcs)?,

        Commands::New { name, lang, path } => cmd_new(name, lang, path)?,

        Commands::Install { projects } => {
            for_each_project("installed", &projects, &[], |b, c| b.install(c))?;
        }

        Commands::Add { dep, project, dev } => {
            let (workspace_root, config) = load_workspace()?;
            let registry = build_registry();
            let proj = config.find_project(&project)?;
            let backend = registry.get(&proj.language)?;
            let ctx = ProjectContext::new(proj.clone(), &workspace_root, vec![]);
            backend.ensure_toolchain(&ctx)?;
            backend.add_dep(&ctx, &dep, dev)?;
            ui::success(&format!("added '{}' to {}", dep, project));
        }

        Commands::Remove { dep, project } => {
            let (workspace_root, config) = load_workspace()?;
            let registry = build_registry();
            let proj = config.find_project(&project)?;
            let backend = registry.get(&proj.language)?;
            let ctx = ProjectContext::new(proj.clone(), &workspace_root, vec![]);
            backend.remove_dep(&ctx, &dep)?;
            ui::success(&format!("removed '{}' from {}", dep, project));
        }

        Commands::Delete { name, yes } => {
            let (workspace_root, mut config) = load_workspace()?;
            let proj = config.find_project(&name)?;
            let dir = workspace_root.join(&proj.path);

            if !yes {
                eprint!("delete project '{}' at {}? [y/N] ", name, proj.path);
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                if !matches!(input.trim().to_lowercase().as_str(), "y" | "yes") {
                    eprintln!("aborted");
                    return Ok(());
                }
            }

            config.remove_project(&name)?;
            config.save(&config_path(&workspace_root))?;

            if dir.exists() {
                std::fs::remove_dir_all(&dir)?;
            }
            ui::success(&format!("deleted project '{}'", name));
        }

        Commands::Build { projects, flags } => {
            for_each_project("built", &projects, &flags, |b, c| b.build(c))?;
        }

        Commands::Run { projects, flags } => {
            for_each_project("ran", &projects, &flags, |b, c| b.run(c))?;
        }

        Commands::Test { projects, flags } => {
            for_each_project("tested", &projects, &flags, |b, c| b.test(c))?;
        }

        Commands::Fmt { projects } => {
            for_each_project("formatted", &projects, &[], |b, c| b.fmt(c))?;
        }

        Commands::Lint { projects } => {
            for_each_project("linted", &projects, &[], |b, c| b.lint(c))?;
        }

        Commands::Clean { projects } => {
            for_each_project("cleaned", &projects, &[], |b, c| b.clean(c))?;
        }

        Commands::Status => cmd_status()?,

        Commands::Exec { project, cmd } => {
            if cmd.is_empty() {
                anyhow::bail!("no command specified — usage: fire exec <project> -- <command>");
            }
            let (workspace_root, config) = load_workspace()?;
            let proj = config.find_project(&project)?;
            let ctx = ProjectContext::new(proj.clone(), &workspace_root, vec![]);
            let args: Vec<&str> = cmd[1..].iter().map(|s| s.as_str()).collect();
            CommandRunner::run(&cmd[0], &args, &ctx.project_root)?;
        }

        Commands::Completions { shell } => {
            match shell {
                Shell::Bash => {
                    // Generate base completions, then append dynamic project completion
                    let mut cmd = Cli::command();
                    generate(Shell::Bash, &mut cmd, "fire", &mut std::io::stdout());
                    // Append a wrapper that injects project names for relevant subcommands
                    print!("{}", BASH_DYNAMIC_COMPLETIONS);
                }
                _ => {
                    let mut cmd = Cli::command();
                    generate(shell, &mut cmd, "fire", &mut std::io::stdout());
                }
            }
        }

        Commands::CompleteProjects => {
            // Print project names one per line (for shell completion scripts)
            if let Ok((_, config)) = load_workspace() {
                for proj in &config.projects {
                    println!("{}", proj.name);
                }
            }
        }
    }

    Ok(())
}
