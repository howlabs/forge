use anyhow::Result;
use clap::{Parser, Subcommand};
use forge_core::event_loop::EventLoop;
use forge_provider::anthropic::AnthropicProvider;
use forge_context::ContextEngine;
use forge_sandbox::Sandbox;
use tracing_subscriber::EnvFilter;

mod exec;

use exec::{run_exec, ExecConfig, ExecResult};

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

        /// Model to use
        #[arg(short, long, default_value = "claude-sonnet-4-20250514")]
        model: String,

        /// Network access mode (off, restricted, full)
        #[arg(long, default_value = "off")]
        network: String,

        /// Resume a task from checkpoint (v0.180.0)
        #[arg(long, value_name = "TASK_ID")]
        resume: Option<String>,
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

        /// Model to use
        #[arg(short, long, default_value = "claude-sonnet-4-20250514")]
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
            model,
            network,
            resume,
        } => {
            tracing::info!("Forge v{} starting (REPL mode)", env!("CARGO_PKG_VERSION"));

            // Handle resume command (v0.180.0)
            if let Some(task_id) = resume {
                return resume_task(&task_id, &project_path);
            }

            // Initialize provider (v0.100.0: Anthropic only)
            let provider = AnthropicProvider::new(api_key, model)?;

            // Initialize context engine (minimal: AGENTS.md loading)
            let context = ContextEngine::new(project_path.clone())?;

            // Initialize sandbox with network-off by default
            let sandbox = Sandbox::new(project_path, network)?;

            // Create and run event loop
            let mut event_loop = EventLoop::new(provider, context, sandbox);
            event_loop.run().await?;

            Ok(())
        }
        Commands::Exec {
            task,
            project_path: _,
            config: _,
            api_key: _,
            model: _,
            verify,
            format,
            trace,
        } => {
            tracing::info!("Forge v{} starting (exec mode)", env!("CARGO_PKG_VERSION"));

            let exec_config = ExecConfig {
                task,
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

/// Resume a task from checkpoint (v0.180.0)
fn resume_task(task_id: &str, project_path: &str) -> Result<()> {
    use forge_agents::{CheckpointStore, Checkpoint};
    use forge_verify::FileCheckpointStore;
    use std::path::PathBuf;

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
