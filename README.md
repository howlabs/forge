# Forge



**An open-source CLI coding agent - 2027-grade successor to Codex CLI, Claude Code, Droid and Augment**

## Vision

Forge is a single Rust binary (no Node/Python runtime) that aims to be lighter than Codex, model-agnostic, with a semantic context engine and parallel multi-agent orchestration with long-horizon endurance.

## Non-Negotiable Principles

1. **One static binary, instant startup** - No Node/Python runtime
2. **Model-agnostic** - `ModelProvider` trait with OpenAI, Anthropic, Gemini, ZAI, and OpenRouter impls
3. **Semantic context** - tree-sitter + knowledge graph + local vector store, NOT grep-everything
4. **Isolated subagent contexts** - Auto git worktrees for safe parallelism
5. **Mandatory verify loop** - Never report done until tests/build pass
6. **Sandbox** - Network-off, dir-scoped, tiered autonomy
7. **Long-horizon** - Checkpoint + resume
8. **Open extensibility** - MCP client/server, hooks, skills, optional AGENTS.md

## Supported Providers

Forge CLI supports multiple AI model providers via the `--provider` flag. The
**default is `zai` with model `glm-5.1`** (Z.AI), as defined by the CLI defaults
in `forge-cli/src/main.rs` and by `forge.toml`.

- **Z.AI** *(default)* — GLM models (`glm-5.1`, `glm-4.7`, `glm-4.5`, `glm-4.5-air`) via the OpenAI-compatible `api.z.ai` endpoint. Requires `ZAI_API_KEY`.
- **Anthropic** — Claude models (e.g. `claude-3-5-sonnet`). Requires `ANTHROPIC_API_KEY`.
- **OpenAI** — GPT models (e.g. `gpt-4o`). Requires `OPENAI_API_KEY`.
- **Gemini** — Google Gemini models (e.g. `gemini-1.5-pro`). Requires `GEMINI_API_KEY`.
- **OpenRouter** — Any model via OpenRouter (e.g. `anthropic/claude-sonnet-4`). Requires `OPENROUTER_API_KEY`.

Both interactive TUI (`forge repl --tui`) and headless execution (`forge exec`) modes fully respect the specified provider and API key configuration options.

## Architecture

```
forge/
├── Cargo.toml              # Workspace configuration (9 crates)
├── forge.toml              # Runtime configuration
├── rust-toolchain.toml     # Pinned toolchain (stable)
├── .github/workflows/      # CI (fmt + clippy + build + offline test)
├── forge-cli/              # CLI entrypoint + binary `forge`
├── forge-core/             # Event loop, diff-edit, streaming
├── provider/               # ModelProvider trait + impls (default: Z.AI)
├── context/                # Semantic context engine (tree-sitter + KG + vectors)
├── agents/                 # Multi-agent orchestration (worktree per subagent)
├── sandbox/                # Safe execution sandbox (network-off, dir-scoped)
├── verify/                 # Verify loop (test/build) + checkpoints
├── ext/                    # Extensions (MCP, hooks, skills, observability)
└── forge-tui/              # Terminal UI (ratatui)
```

## Roadmap vs. reality

Forge's original roadmap was staged across `v0.100.0` → `v0.190.0`. In practice
most subsystems have already been **scaffolded with passing tests**, but not all
are wired into the running binary end-to-end. The table below is the honest
per-capability status as of this build (verified by `cargo build` + `cargo
test`; see `docs.md` for the reference design).

Legend: ✅ working · 🟡 partial · ⚪ scaffold-only (compiles, tests pass, not
exercised by the live binary).

| Capability | Crate(s) | Status | Notes |
| --- | --- | --- | --- |
| CLI + tool→observe→act event loop | forge-cli, forge-core | ✅ | TUI mode is interactive; headless exec supports all configured providers. |
| `ModelProvider` trait + impls | provider | ✅ | Anthropic, OpenAI, Z.AI (default), Gemini, OpenRouter. |
| File read / diff-edit / run-command | forge-core | ✅ | `tool_read_file`/`tool_write_file`/`tool_diff_edit`/`tool_run_command` exercised via 15 event-loop tests (MockProvider). |
| Sandbox (network-off, dir-scoped) | sandbox | ✅ | path-traversal guards; 5 tests. Plain `repl` prints a stub message. |
| AGENTS.md loading (layered) | context | ✅ | bounded + unbounded discovery; 79 tests |
| Semantic context engine (tree-sitter + KG + vector store) | context | 🟡 | 80 tests; `ContextEngine` used in CLI, but `ContextIndex` (tree-sitter + KG + vector) is opt-in via `.with_context_index()`, not the default retrieval path |
| Multi-agent orchestrator + git worktree | agents | ✅ | spawn/join, worktree-per-task; 10 tests |
| Checkpoint / resume | verify, forge-cli, forge-core | ✅ | `CheckpointState` + `FileCheckpointStore`; `--resume` restores history/worktree and continues |
| Verify loop (build/test) | verify | ✅ | `BuildVerifier`; 4 tests |
| Extensions: MCP / hooks / skills / observability | ext | 🟡 | `trace.jsonl` + `forge logs --last` wired; MCP/hooks/skills still scaffold |
| TUI | forge-tui | ✅ | Real-time streaming, live token/cost bar, interactive approval `[y/n/a]`, `--yolo` |
| `forge exec` headless | forge-cli | ✅ | fully functional; respects all provider configuration flags. |

### Original milestone plan (aspirational)

The version milestones below remain useful as a forward backlog. They are
**not** a description of what each tag currently ships.

- **v0.100.0** — Rust workspace + event loop + one provider + edit/run + sandbox + AGENTS.md
- **v0.130.0** — Context engine with tree-sitter index + knowledge graph + local vector store
- **v0.150.0** — Incremental sync (file watcher) + verify-symbol-before-edit
- **v0.170.0** — Agents orchestrator + isolated subagents + auto git worktree
- **v0.180.0** — Checkpoint/resume + mandatory verify loop
- **v0.190.0** — MCP client/server + hooks + skills + `forge exec` headless + multi-provider + observability

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

This produces the `forge` binary at `target/release/forge` (or
`target\release\forge.exe` on Windows).

> **Windows developers without Visual Studio C++ Build Tools:** the default
> `msvc` target needs `link.exe`, which only ships with the VS C++ Build Tools.
> Two workarounds are documented in `rust-toolchain.toml`: build with the
> `x86_64-pc-windows-gnu` toolchain, and — if your machine applies Application
> Control / Device Guard policy to the `Desktop` tree — point
> `CARGO_TARGET_DIR` at a directory outside `%USERPROFILE%\Desktop`.

## Usage

```bash
# Set your API key. The default provider is Z.AI, so use ZAI_API_KEY:
export ZAI_API_KEY="your-zai-api-key"

# Run Forge in the current directory (default provider/model: zai / glm-5.1)
cargo run -- repl --project-path . --api-key $ZAI_API_KEY

# Use a different provider explicitly:
cargo run -- repl --provider anthropic --model claude-sonnet-4-20250514 --api-key $ANTHROPIC_API_KEY

# Or with the compiled binary:
./target/release/forge repl --project-path . --api-key $ZAI_API_KEY
```

## Configuration

Edit `forge.toml` to configure providers, sandbox settings, and verify loop.
The committed default is `type = "zai"`, `model = "glm-5.1"`,
`api_key_env = "ZAI_API_KEY"`.

## License

MIT OR Apache-2.0

## Status

**Version**: 0.100.0 (MVP)

This is an early MVP. What is true of this build:

- ✅ The full 9-crate workspace **compiles** (`cargo build --workspace --all-targets`) and **links a single `forge` binary** (`cargo build --release`).
- ✅ `cargo clippy --workspace --all-targets -- -D warnings` is clean.
- ✅ `cargo fmt --all -- --check` is clean.
- ✅ `cargo test --workspace` is **green and fully offline** (336 tests; 1 ignored integration test gated behind the `integration` feature).
- ✅ CI gates fmt + clippy + build + offline test on ubuntu, macOS, and Windows. See `.github/workflows/ci.yml`.

What is **not** true yet (honest gaps):

- The interactive `repl` plain mode prints a welcome/stub message; the TUI provides a functional environment with conversation/diff/agent panels and runs the event loop end-to-end, but does not yet support real-time token/cost streaming or interactive human-in-the-loop approvals.
- Extensions (MCP/hooks/skills/observability) compile and have 152 passing unit tests, but are not wired into the CLI or TUI.
- The semantic context engine's `ContextIndex` (tree-sitter + KG + vector store) is opt-in via `.with_context_index()` — not the default retrieval path in the live binary.
- Default provider is **Z.AI / glm-5.1** (not Anthropic and not OpenAI), per the code.

See `docs.md` for reference design specifications.
