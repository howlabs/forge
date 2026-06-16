use anyhow::Result;
use clap::{Parser, Subcommand};
use context::ContextEngine;
use forge_tui::{SimpleTui, TuiConfig};
use provider::ModelProvider;
use sandbox::Sandbox;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

use agents::Orchestrator;

mod doctor;
mod exec;
mod mcp;

use exec::{run_exec, ExecConfig};

// ProviderType enum removed in favor of registry

/// Forge - An open-source CLI coding agent
#[derive(Parser, Debug)]
#[command(name = "forge")]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Interactive REPL mode (default)
    Repl {
        /// Path to the project directory
        #[arg(short, long, default_value = ".")]
        project_path: String,

        /// Configuration file path
        #[arg(short, long, default_value = "forge.toml")]
        config: String,

        /// API key for the model provider. Optional for the `mock`/`local`
        /// providers and when the provider's env var is set.
        #[arg(short, long)]
        api_key: Option<String>,

        /// Model provider (anthropic, openai, zai, gemini, local, mock)
        #[arg(long, default_value = "zai")]
        provider: String,

        /// Model to use
        #[arg(short, long, default_value = "glm-5.1")]
        model: String,

        /// Network access mode (off, on, auto)
        #[arg(short, long, default_value = "off")]
        network: String,

        /// Watch project files for changes and incrementally update the semantic index
        #[arg(short, long)]
        watch: bool,

        /// Resume a task from checkpoint (v0.180.0)
        #[arg(long, value_name = "TASK_ID")]
        resume: Option<String>,

        /// Launch TUI mode (interactive terminal UI)
        #[arg(long, default_value_t = false)]
        tui: bool,

        /// Force plain mode even when in TTY
        #[arg(long, default_value_t = false)]
        plain: bool,
    },
    /// Headless exec mode for CI/CD
    Exec {
        /// Task description (positional)
        task: Option<String>,

        /// Task description (alias for positional task)
        #[arg(long)]
        prompt: Option<String>,

        /// Path to the project directory
        #[arg(short, long, default_value = ".")]
        project_path: String,

        /// Configuration file path
        #[arg(short, long, default_value = "forge.toml")]
        config: String,

        /// API key for the model provider. Optional for the `mock`/`local`
        /// providers and when the provider's env var is set.
        #[arg(short, long)]
        api_key: Option<String>,

        /// Model provider (anthropic, openai, zai, gemini, local, mock)
        #[arg(long, default_value = "zai")]
        provider: String,

        /// Model to use
        #[arg(short, long, default_value = "glm-5.1")]
        model: String,

        /// Run verify loop
        #[arg(long, action = clap::ArgAction::Set, default_value_t = true)]
        verify: bool,

        /// Output format
        #[arg(long, default_value_t = String::from("json"))]
        format: String,

        /// Enable trace logging
        #[arg(long, default_value_t = false)]
        trace: bool,
    },
    /// Multi-agent orchestration: spawn, list, and join subagents
    Agents {
        #[command(subcommand)]
        action: AgentAction,
    },
    /// Diagnose the environment: provider keys, git, cargo, sandbox, config
    Doctor {
        /// Path to the project directory
        #[arg(short, long, default_value = ".")]
        project_path: String,

        /// Configuration file path
        #[arg(short, long, default_value = "forge.toml")]
        config: String,

        /// Network access mode to report on (off, on, auto)
        #[arg(short, long, default_value = "off")]
        network: String,

        /// Emit machine-readable JSON instead of the text report
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Model Context Protocol: run Forge as an MCP stdio server
    Mcp {
        #[command(subcommand)]
        action: McpAction,
    },
}

#[derive(Subcommand, Debug)]
enum McpAction {
    /// Serve Forge's built-in tools over MCP stdio (line-delimited JSON-RPC)
    Serve {
        /// Path to the project directory exposed to MCP tools
        #[arg(short, long, default_value = ".")]
        project_path: String,

        /// Network access mode for sandbox-backed tools (off, on, auto)
        #[arg(short, long, default_value = "off")]
        network: String,
    },
}

#[derive(Subcommand, Debug)]
enum AgentAction {
    /// Spawn a new subagent to work on a task
    Spawn {
        /// Task description / prompt for the subagent
        #[arg(long)]
        prompt: String,

        /// Model provider
        #[arg(long, default_value = "zai")]
        provider: String,

        /// Model to use
        #[arg(short, long, default_value = "glm-5.1")]
        model: String,

        /// API key for the provider
        #[arg(short, long)]
        api_key: String,

        /// File scope globs (comma-separated, e.g. "src/**,tests/**")
        #[arg(short, long)]
        scope: Option<String>,

        /// Path to the project directory
        #[arg(short, long, default_value = ".")]
        project_path: String,
    },
    /// List all agents and their status
    List {
        /// Path to the project directory
        #[arg(short, long, default_value = ".")]
        project_path: String,
    },
    /// Wait for all running agents to complete and merge their changes
    Join {
        /// Path to the project directory
        #[arg(short, long, default_value = ".")]
        project_path: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Repl {
            project_path,
            config: _,
            api_key,
            provider,
            model,
            network,
            watch,
            resume,
            tui,
            plain,
        } => {
            tracing::info!("Forge v{} starting (REPL mode)", env!("CARGO_PKG_VERSION"));

            // Handle resume command (v0.180.0)
            if let Some(task_id) = resume {
                return resume_task(&task_id, &project_path);
            }

            let api_key = resolve_api_key(&provider, api_key)?;

            // Provider will be resolved via registry

            // Initialize context engine (minimal: AGENTS.md loading)
            let context = ContextEngine::new(project_path.clone())?;

            // Initialize sandbox with network-off by default
            let sandbox = Sandbox::new(project_path.clone(), network)?;

            let context = std::sync::Arc::new(tokio::sync::Mutex::new(context));

            let mut _watcher = None;
            if watch {
                let mut w = forge_core::file_watcher::FileWatcher::new(
                    context.clone()
                        as std::sync::Arc<tokio::sync::Mutex<dyn context::ContextIndex>>,
                    &project_path,
                    500, // debounce
                )?;
                w.watch()?;
                _watcher = Some(w);
                tracing::info!("File watcher started for incremental indexing");
            }

            // Determine mode: TUI, plain, or auto-detect
            let is_tty = atty::is(atty::Stream::Stdout);
            let use_tui = tui || (is_tty && !plain);

            if use_tui {
                tracing::info!("Launching TUI mode with {} provider", provider);
                launch_tui_mode(
                    provider.clone(),
                    model,
                    api_key,
                    context,
                    sandbox,
                    project_path,
                )
                .await?;
            } else {
                tracing::info!("Launching plain REPL mode with {} provider", provider);
                launch_plain_mode(provider.clone(), model, api_key, context, sandbox).await?;
            }

            Ok(())
        }
        Commands::Exec {
            task,
            prompt,
            project_path,
            config,
            api_key,
            provider,
            model,
            verify,
            format,
            trace: _,
        } => {
            tracing::info!("Forge v{} starting (exec mode)", env!("CARGO_PKG_VERSION"));

            let api_key = resolve_api_key(&provider, api_key)?;

            let task = task.or(prompt).ok_or_else(|| {
                anyhow::anyhow!(
                    "Missing task description. Pass it as a positional argument or via --prompt."
                )
            })?;

            let exec_config = ExecConfig {
                task,
                project_path: std::path::PathBuf::from(project_path),
                config_path: std::path::PathBuf::from(config),
                api_key,
                provider,
                model,
                verify,
            };

            let result = run_exec(exec_config).await?;

            // Print result in requested format
            if format == "json" {
                println!("{}", result.to_json()?);
            } else {
                println!("{}", result.to_text());
            }

            std::process::exit(result.exit_code());
        }
        Commands::Agents { action } => {
            match action {
                AgentAction::Spawn {
                    prompt,
                    provider: provider_name,
                    model,
                    api_key,
                    scope,
                    project_path,
                } => {
                    let project_path = std::path::PathBuf::from(&project_path);
                    let worktree_base = project_path.join(".forge").join("worktrees");

                    let mut orchestrator = agents::MultiAgentOrchestrator::new(
                        &project_path,
                        &worktree_base,
                        4, // max parallel
                    )?;

                    let mut task = agents::Task::new(&prompt, std::path::PathBuf::new())
                        .with_provider(api_key, model)
                        .with_provider_name(&provider_name);

                    if let Some(scope_str) = scope {
                        let globs: Vec<String> =
                            scope_str.split(',').map(|s| s.trim().to_string()).collect();
                        task = task.with_scope(globs);
                    }

                    let task_id = task.id.clone();
                    orchestrator.spawn(task).await?;
                    println!("Spawned agent {}", task_id);
                    println!("Use `forge agents join` to wait for completion and merge.");

                    // Wait for completion
                    let completed = orchestrator.join_all().await?;
                    for t in &completed {
                        let status = if t.status == agents::TaskStatus::Done {
                            "DONE"
                        } else {
                            "FAILED"
                        };
                        println!(
                            "[{}] {} — {}",
                            status,
                            t.id,
                            t.result.as_deref().unwrap_or("")
                        );
                    }
                    Ok(())
                }
                AgentAction::List { project_path } => {
                    let project_path = std::path::PathBuf::from(&project_path);
                    let worktree_base = project_path.join(".forge").join("worktrees");

                    let orchestrator =
                        agents::MultiAgentOrchestrator::new(&project_path, &worktree_base, 4)?;

                    let tasks = orchestrator.list_tasks_from_disk();
                    if tasks.is_empty() {
                        println!("No agents found.");
                    } else {
                        println!(
                            "{:<12} {:<10} {:<40} {}",
                            "ID", "STATUS", "PROMPT", "RESULT"
                        );
                        println!("{}", "-".repeat(80));
                        for t in &tasks {
                            let status = format!("{:?}", t.status);
                            let prompt = if t.prompt.chars().count() > 37 {
                                let truncated: String = t.prompt.chars().take(37).collect();
                                format!("{truncated}...")
                            } else {
                                t.prompt.clone()
                            };
                            let result = t.result.as_deref().unwrap_or("");
                            println!(
                                "{:<12} {:<10} {:<40} {}",
                                &t.id[..12],
                                status,
                                prompt,
                                result
                            );
                        }
                    }
                    Ok(())
                }
                AgentAction::Join { project_path } => {
                    let project_path = std::path::PathBuf::from(&project_path);
                    let worktree_base = project_path.join(".forge").join("worktrees");

                    let orchestrator =
                        agents::MultiAgentOrchestrator::new(&project_path, &worktree_base, 4)?;

                    // ponytail: can't resume across processes.  Report
                    // status of persisted tasks.  Merges were already
                    // handled by the original `spawn` invocation.
                    let tasks = orchestrator.list_tasks_from_disk();
                    if tasks.is_empty() {
                        println!("No agents found.");
                    } else {
                        for t in &tasks {
                            let status = if t.status == agents::TaskStatus::Done {
                                "DONE"
                            } else if t.status == agents::TaskStatus::Failed {
                                "FAILED"
                            } else {
                                "RUNNING"
                            };
                            println!(
                                "[{}] {} — {}",
                                status,
                                t.id,
                                t.result.as_deref().unwrap_or("in progress")
                            );
                        }
                    }
                    Ok(())
                }
            }
        }
        Commands::Doctor {
            project_path,
            config,
            network,
            json,
        } => {
            let exit_code = doctor::run(&doctor::DoctorConfig {
                project_path,
                config_path: config,
                network,
                json,
            });
            std::process::exit(exit_code);
        }
        Commands::Mcp { action } => match action {
            McpAction::Serve {
                project_path,
                network,
            } => {
                tracing::info!("Starting Forge MCP stdio server");
                mcp::serve(project_path, network).await
            }
        },
    }
}

pub fn create_provider_instance(
    name: &str,
    model: &str,
    api_key: &str,
) -> Result<Arc<dyn ModelProvider>> {
    provider::create_provider(name, model, api_key)
}

/// Resolve the API key for a provider from, in order: the explicit `--api-key`
/// flag, the provider's registered environment variable, or an empty string for
/// providers that need no credentials (`mock`/`local`). Errors when a key is
/// genuinely required but missing.
fn resolve_api_key(provider_name: &str, explicit: Option<String>) -> Result<String> {
    if let Some(key) = explicit.filter(|k| !k.is_empty()) {
        return Ok(key);
    }

    let lower = provider_name.to_lowercase();
    if lower == "mock" || lower == "local" {
        return Ok(String::new());
    }

    // Provider-specific env var from the registry, with sensible fallbacks for
    // the providers that have dedicated constructors.
    let env_var = provider::find_provider(&lower)
        .map(|entry| entry.env_var.to_string())
        .unwrap_or_else(|| match lower.as_str() {
            "anthropic" => "ANTHROPIC_API_KEY".to_string(),
            "gemini" => "GEMINI_API_KEY".to_string(),
            _ => "FORGE_API_KEY".to_string(),
        });

    if let Ok(key) = std::env::var(&env_var) {
        if !key.is_empty() {
            return Ok(key);
        }
    }

    anyhow::bail!(
        "No API key for provider '{}'. Pass --api-key or set {}. \
         (Use --provider mock for offline runs.)",
        provider_name,
        env_var
    )
}

/// Launch TUI mode with configured provider
async fn launch_tui_mode(
    provider_name: String,
    model: String,
    api_key: String,
    _context: std::sync::Arc<tokio::sync::Mutex<context::ContextEngine>>,
    _sandbox: Sandbox,
    project_path: String,
) -> Result<()> {
    // Create provider based on registry
    let provider = create_provider_instance(&provider_name, &model, &api_key)?;

    // Initialize TUI configuration
    let config = TuiConfig {
        fullscreen: false,
        show_agent_panel: true,
        ..Default::default()
    };

    tracing::info!("Starting TUI with {} provider", provider_name);
    tracing::info!("Project path: {}", project_path);

    // Create and run TUI
    let mut tui = SimpleTui::with_event_loop(config, provider);
    tui.run().await?;

    Ok(())
}

/// Launch plain REPL mode with configured provider
async fn launch_plain_mode(
    provider_name: String,
    model: String,
    api_key: String,
    context: std::sync::Arc<tokio::sync::Mutex<context::ContextEngine>>,
    sandbox: Sandbox,
) -> Result<()> {
    let _ = (provider_name, model, api_key, context, sandbox);
    println!(
        "Plain REPL mode is not interactive yet. Use --tui for the EventLoop-backed TUI or `forge exec` for headless tasks."
    );
    Ok(())
}

/// Resume a task from checkpoint (v0.180.0)
fn resume_task(task_id: &str, project_path: &str) -> Result<()> {
    use std::path::PathBuf;
    use verify::FileCheckpointStore;

    tracing::info!("Resuming task {} from checkpoint", task_id);

    let store_path = PathBuf::from(project_path).join(".forge/checkpoints");
    let store = FileCheckpointStore::new(store_path)?;

    // Load checkpoint using sync wrapper
    let checkpoint = store.load_sync(task_id)?;

    match checkpoint {
        Some(checkpoint) => {
            tracing::info!(
                "Found checkpoint for task {} at step {}",
                task_id,
                checkpoint.step
            );
            tracing::info!("State size: {} bytes", checkpoint.state.len());
            tracing::info!("Timestamp: {:?}", checkpoint.timestamp);

            // TODO: Restore state and continue execution
            // For v0.180.0 MVP, just display checkpoint info
            println!("Task {} resumed from step {}", task_id, checkpoint.step);
            println!("State size: {} bytes", checkpoint.state.len());
            println!("To continue: Implement state restoration and execution");

            Ok(())
        }
        None => {
            tracing::error!("No checkpoint found for task {}", task_id);
            anyhow::bail!("No checkpoint found for task {}", task_id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parsing() {
        let cli = Cli::try_parse_from(["forge", "repl", "--api-key", "test"]);
        assert!(cli.is_ok());
    }

    #[test]
    fn test_exec_parsing() {
        let cli = Cli::try_parse_from(["forge", "exec", "test task", "--api-key", "test"]);
        assert!(cli.is_ok());
    }

    #[test]
    fn test_agents_spawn_parsing() {
        let cli = Cli::try_parse_from([
            "forge",
            "agents",
            "spawn",
            "--prompt",
            "fix the bug",
            "--api-key",
            "test",
            "--scope",
            "src/**",
        ]);
        assert!(cli.is_ok());
    }

    #[test]
    fn test_agents_list_parsing() {
        let cli = Cli::try_parse_from(["forge", "agents", "list"]);
        assert!(cli.is_ok());
    }

    #[test]
    fn test_agents_join_parsing() {
        let cli = Cli::try_parse_from(["forge", "agents", "join"]);
        assert!(cli.is_ok());
    }
}
