use anyhow::Result;
use clap::Parser;
use forge_core::event_loop::EventLoop;
use forge_provider::anthropic::AnthropicProvider;
use forge_context::ContextEngine;
use forge_sandbox::Sandbox;
use tracing_subscriber::EnvFilter;

/// Forge - An open-source CLI coding agent
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
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
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    tracing::info!("Forge v{} starting", env!("CARGO_PKG_VERSION"));

    // Handle resume command (v0.180.0)
    if let Some(task_id) = args.resume {
        return resume_task(&task_id, &args.project_path);
    }

    // Initialize provider (v0.100.0: Anthropic only)
    let provider = AnthropicProvider::new(args.api_key, args.model)?;

    // Initialize context engine (minimal: AGENTS.md loading)
    let context = ContextEngine::new(args.project_path.clone())?;

    // Initialize sandbox with network-off by default
    let sandbox = Sandbox::new(args.project_path, args.network)?;

    // Create and run event loop
    let mut event_loop = EventLoop::new(provider, context, sandbox);
    event_loop.run().await?;

    Ok(())
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
