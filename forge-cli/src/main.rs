use anyhow::Result;
use clap::{Parser, Subcommand};
use context::ContextEngine;
use forge_tui::{SimpleTui, TuiConfig};
use provider::anthropic::AnthropicProvider;
use provider::gemini::GeminiProvider;
use provider::local::LocalProvider;
use provider::openai::OpenAIProvider;
use provider::ModelProvider;
use sandbox::Sandbox;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

mod exec;

use exec::{run_exec, ExecConfig};

/// Supported model providers
#[derive(Debug, Clone, PartialEq)]
enum ProviderType {
    Anthropic,
    OpenAI,
    Zai,
    Gemini,
    Local,
}

impl std::str::FromStr for ProviderType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "anthropic" => Ok(ProviderType::Anthropic),
            "openai" => Ok(ProviderType::OpenAI),
            "zai" | "z.ai" | "glm" => Ok(ProviderType::Zai),
            "gemini" => Ok(ProviderType::Gemini),
            "local" => Ok(ProviderType::Local),
            _ => anyhow::bail!("Unknown provider: {}", s),
        }
    }
}

impl std::fmt::Display for ProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderType::Anthropic => write!(f, "anthropic"),
            ProviderType::OpenAI => write!(f, "openai"),
            ProviderType::Zai => write!(f, "zai"),
            ProviderType::Gemini => write!(f, "gemini"),
            ProviderType::Local => write!(f, "local"),
        }
    }
}

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

        /// API key for the model provider
        #[arg(short, long)]
        api_key: String,

        /// Model provider (anthropic, openai, zai, gemini, local)
        #[arg(long, default_value = "zai")]
        provider: String,

        /// Model to use
        #[arg(short, long, default_value = "glm-5.1")]
        model: String,

        /// Network access mode (off, restricted, full)
        #[arg(long, default_value = "off")]
        network: String,

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
        /// Task description
        task: String,

        /// Path to the project directory
        #[arg(short, long, default_value = ".")]
        project_path: String,

        /// Configuration file path
        #[arg(short, long, default_value = "forge.toml")]
        config: String,

        /// API key for the model provider
        #[arg(short, long)]
        api_key: String,

        /// Model provider (anthropic, openai, zai, gemini, local)
        #[arg(long, default_value = "zai")]
        provider: String,

        /// Model to use
        #[arg(short, long, default_value = "glm-5.1")]
        model: String,

        /// Run verify loop
        #[arg(long, default_value_t = true)]
        verify: bool,

        /// Output format
        #[arg(long, default_value_t = String::from("json"))]
        format: String,

        /// Enable trace logging
        #[arg(long, default_value_t = false)]
        trace: bool,
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
            resume,
            tui,
            plain,
        } => {
            tracing::info!("Forge v{} starting (REPL mode)", env!("CARGO_PKG_VERSION"));

            // Handle resume command (v0.180.0)
            if let Some(task_id) = resume {
                return resume_task(&task_id, &project_path);
            }

            // Parse provider type
            let provider_type = provider.parse::<ProviderType>()?;

            // Initialize context engine (minimal: AGENTS.md loading)
            let context = ContextEngine::new(project_path.clone())?;

            // Initialize sandbox with network-off by default
            let sandbox = Sandbox::new(project_path.clone(), network)?;

            // Determine mode: TUI, plain, or auto-detect
            let is_tty = atty::is(atty::Stream::Stdout);
            let use_tui = tui || (is_tty && !plain);

            if use_tui {
                tracing::info!("Launching TUI mode with {} provider", provider);
                launch_tui_mode(
                    provider_type,
                    model,
                    api_key,
                    context,
                    sandbox,
                    project_path,
                )
                .await?;
            } else {
                tracing::info!("Launching plain REPL mode with {} provider", provider);
                launch_plain_mode(provider_type, model, api_key, context, sandbox).await?;
            }

            Ok(())
        }
        Commands::Exec {
            task,
            project_path,
            config,
            api_key,
            provider,
            model,
            verify,
            format,
            trace,
        } => {
            tracing::info!("Forge v{} starting (exec mode)", env!("CARGO_PKG_VERSION"));

            let exec_config = ExecConfig {
                task,
                project_path: std::path::PathBuf::from(project_path),
                config_path: std::path::PathBuf::from(config),
                api_key,
                provider,
                model,
                verify,
                output_format: format.clone(),
                trace,
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
    }
}

/// Launch TUI mode with configured provider
async fn launch_tui_mode(
    provider_type: ProviderType,
    model: String,
    api_key: String,
    _context: ContextEngine,
    _sandbox: Sandbox,
    project_path: String,
) -> Result<()> {
    // Create provider based on type
    let provider: Arc<dyn ModelProvider> = match provider_type {
        ProviderType::Anthropic => {
            tracing::info!("Creating Anthropic provider with model: {}", model);
            Arc::new(AnthropicProvider::new(api_key, model)?)
        }
        ProviderType::OpenAI => {
            tracing::info!("Creating OpenAI provider with model: {}", model);
            Arc::new(OpenAIProvider::new(model, api_key))
        }
        ProviderType::Zai => {
            tracing::info!("Creating Z.AI provider with model: {}", model);
            Arc::new(OpenAIProvider::with_base_url(
                model,
                api_key,
                "https://api.z.ai/api/coding/paas/v4/chat/completions",
            ))
        }
        ProviderType::Gemini => {
            tracing::info!("Creating Gemini provider with model: {}", model);
            Arc::new(GeminiProvider::new(model, api_key))
        }
        ProviderType::Local => {
            tracing::info!("Creating Local provider with model: {}", model);
            Arc::new(LocalProvider::new_ollama("http://localhost:11434", model))
        }
    };

    // Initialize TUI configuration
    let config = TuiConfig {
        fullscreen: false,
        show_agent_panel: true,
        ..Default::default()
    };

    tracing::info!("Starting TUI with {} provider", provider_type);
    tracing::info!("Project path: {}", project_path);

    // Create and run TUI
    let mut tui = SimpleTui::with_event_loop(config, provider);
    tui.run().await?;

    Ok(())
}

/// Launch plain REPL mode with configured provider
async fn launch_plain_mode(
    provider_type: ProviderType,
    model: String,
    api_key: String,
    context: ContextEngine,
    sandbox: Sandbox,
) -> Result<()> {
    let _ = (provider_type, model, api_key, context, sandbox);
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
}
