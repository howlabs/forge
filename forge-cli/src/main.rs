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
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    tracing::info!("Forge v{} starting", env!("CARGO_PKG_VERSION"));

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
