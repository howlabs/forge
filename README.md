# Forge

**An open-source CLI coding agent - 2027-grade successor to Codex CLI, Claude Code, Droid and Augment**

## Vision

Forge is a single static Rust binary that is lighter than Codex, model-agnostic + local-first, with a semantic context engine and parallel multi-agent orchestration with long-horizon endurance.

## Non-Negotiable Principles

1. **One static binary, instant startup** - No Node/Python runtime
2. **Model-agnostic** - `ModelProvider` trait with OpenAI, Anthropic, Gemini and local (Ollama/llama.cpp) impls
3. **Semantic context** - tree-sitter + knowledge graph + local vector store, NOT grep-everything
4. **Isolated subagent contexts** - Auto git worktrees for safe parallelism
5. **Mandatory verify loop** - Never report done until tests/build pass
6. **Sandbox** - Network-off, dir-scoped, tiered autonomy
7. **Long-horizon** - Checkpoint + resume
8. **Open extensibility** - MCP client/server, hooks, skills, optional AGENTS.md

## Supported Providers

Forge CLI supports multiple AI model providers:

- **OpenAI** - GPT-4, GPT-3.5 (default)
- **Anthropic** - Claude models
- **Gemini** - Google Gemini models
- **Local** - Local models (Ollama/llama.cpp)
- **Z.AI** - GLM models (glm-5.1, glm-4.7, glm-4.5, glm-4.5-air)

See [docs/providers/](docs/providers/) for detailed documentation.

## Architecture

```
forge/
├── Cargo.toml              # Workspace configuration
├── forge.toml              # Runtime configuration
├── forge-cli/              # CLI interface
├── forge-core/             # Event loop, diff-edit, streaming
├── forge-provider/         # ModelProvider trait + implementations
├── forge-context/          # Semantic context engine
├── forge-agents/           # Multi-agent orchestration
├── forge-sandbox/          # Safe execution sandbox
├── forge-verify/           # Verify loop (test/build)
└── forge-ext/              # Extensions (MCP, hooks, skills)
```

## Roadmap

### v0.98.0 (Current - MVP)
- ✅ CLI + core loop (tool->observe->act)
- ✅ Provider with ONE provider (Anthropic)
- ✅ File read/diff-edit + run command
- ✅ Sandbox network-off
- ✅ Load AGENTS.md

### v0.100.0
- Context engine with tree-sitter index
- Knowledge graph
- Local vector store
- Semantic retrieval replaces grep

### v0.130.0
- Incremental sync (file watcher)
- Verify-symbol-before-edit

### v0.150.0
- Agents orchestrator
- Isolated subagents
- Auto git worktree per subagent

### v0.170.0
- Checkpoint/resume
- Mandatory verify loop (test/build)

### v0.180.0
- MCP client/server
- Hooks system
- Skills framework
- `forge exec` headless
- Multi-provider including local
- Basic observability

## Building

```bash
cargo build --release
```

This creates a static binary at `target/release/forge`.

## Usage

```bash
# Set your API key
export FORGE_API_KEY="your-anthropic-api-key"

# Run Forge in current directory
cargo run -- --project-path . --api-key $FORGE_API_KEY

# Or with the compiled binary
./target/release/forge --project-path . --api-key $FORGE_API_KEY
```

## Configuration

Edit `forge.toml` to configure providers, sandbox settings, and verify loop.

## License

MIT OR Apache-2.0

## Status

**Version**: 0.100.0 (MVP)

This is the initial MVP release. See roadmap for future milestones.
