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

### v0.100.0 (Current - MVP)
- ✅ CLI + core loop (tool->observe->act)
- ✅ Provider with ONE provider (Anthropic)
- ✅ File read/diff-edit + run command
- ✅ Sandbox network-off
- ✅ Load AGENTS.md

### v0.130.0
- Context engine with tree-sitter index
- Knowledge graph
- Local vector store
- Semantic retrieval replaces grep

### v0.150.0
- Incremental sync (file watcher)
- Verify-symbol-before-edit

### v0.170.0
- Agents orchestrator
- Isolated subagents
- Auto git worktree per subagent

### v0.180.0
- Checkpoint/resume
- Mandatory verify loop (test/build)

### v0.190.0
- MCP client/server
- Hooks system
- Skills framework
- `forge exec` headless
- Multi-provider including local
- Basic observability

## Versioning

Forge follows Semantic Versioning (`MAJOR.MINOR.PATCH`) with explicit `v`-prefixed release tags.

- Use plain versions in package metadata, for example `0.100.0` in `Cargo.toml`.
- Use `v` prefixes for Git tags, release headings, and roadmap milestones, for example `v0.100.0`.
- While Forge is in the `0.x` phase, increment `MINOR` for milestone releases, user-visible features, or contract changes.
- Increment `PATCH` for compatible bug fixes, documentation updates, test improvements, and internal refactors that do not change behavior.
- Document every release in `CHANGELOG.md`; breaking changes before `v1.0.0` must still be called out explicitly.
- Keep all workspace crates on the same Forge version unless a crate is intentionally split into an independently released package.


## P0 Hardening

Forge now includes the first production-hardening layer for daily CLI use:

- `forge exec` uses the requested `--provider`, `--model`, `--api-key`, `--project-path`, and `--config` values instead of falling back to a hard-coded provider.
- Verification can be configured in `forge.toml` with explicit commands, or auto-detected for Rust, Node, Python, Go, and Make-based projects.
- The command sandbox rejects obvious destructive commands and network tools when network mode is `off`.

Example `forge.toml`:

```toml
[verify]
commands = ["cargo build --quiet", "cargo test --quiet"]
auto_detect = true
max_retries = 5
```

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
