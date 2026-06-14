# Forge TUI

Fast, keyboard-driven terminal UI for Forge CLI.

## Features

- **Streaming Conversation**: Real-time output with markdown rendering
- **Diff Viewer**: Best-in-class diff viewer with approve/reject/edit per hunk
- **Agent Activity Panel**: Live status of parallel subagents (Forge's differentiator)
- **Command Palette**: Keyboard-driven commands (/model, /context, /agents, etc.)
- **Plan/Build Toggle**: Tab key to toggle between plan and build modes
- **Checkpoint/Resume**: Long-horizon task resumption
- **Hybrid Render**: Native terminal scrollback + ratatui overlays
- **Responsive UI**: <16ms render time, never blocks UI thread

## Usage

The TUI is integrated into the Forge CLI. Run:

```bash
forge repl --api-key YOUR_KEY --tui    # Start TUI mode with real AI
forge repl --api-key YOUR_KEY         # Auto-detect (TUI if terminal, plain otherwise)
forge repl --api-key YOUR_KEY --plain # Force plain mode
forge exec                              # Run without TUI (plain mode for CI/pipe)
```

## Keyboard Shortcuts

### Global
- `q` / `Ctrl-C` / `Esc`: Quit
- `Tab`: Toggle Plan/Build mode
- `Enter-Enter`: Queue message (soft steer when agent busy)
- `Esc-Esc`: Hard interrupt current operation

### Navigation
- `↑`/`↓`: Navigate in panels
- `Ctrl-↑`/`Ctrl-↓`: Navigate command history

### Diff Viewer
- `↑`/`↓`: Navigate hunks
- `Enter`: Approve selected hunk
- `r`: Reject selected hunk
- `e`: Edit selected hunk
- `A`: Approve all
- `R`: Reject all

### Commands (Type in input box)
- `/model <name>`: Change AI model
- `/context <add|remove|list> [path]`: Manage context
- `/agents <list|kill> [id]`: Manage parallel agents
- `/resume <task_id>`: Resume from checkpoint
- `/diff [path]`: Show diff viewer
- `/plan`: Enter plan mode
- `/review [path]`: Request code review
- `/init`: Initialize Forge in current directory
- `/help`: Show help

## Architecture

The TUI is a **thin client** over existing Forge traits:
- `agents`: Task, TaskStatus, Orchestrator, Verifier, CheckpointStore
- `forge-core`: EventLoop, ModelProvider
- `verify`: VerifyLoop, CheckpointStore

This keeps business logic in the core and the TUI focused on display/interaction.

## Panels

### Conversation Panel
Displays streaming output with markdown rendering and syntax highlighting.

### Input Box
Multiline input with:
- Command history (Ctrl-↑/↓)
- Slash commands (/)
- Message queue for steering

### Diff Viewer
Best-in-class diff viewer with:
- Syntax-highlighted changes
- Per-hunk approve/reject/edit
- Integration with verify-symbol

### Agent Activity Panel
Live status of parallel subagents:
- Agent ID and task
- Current state (Pending/Running/Verifying/Done/Failed)
- Worktree path
- Progress indicators

### Status Bar
Provider + model, token usage, autonomy tier, verify state, elapsed time, cost.

### Checkpoint Banner
Shows when long-horizon resume is available (press 'R' to resume).

## Status

- [x] Complete TUI structure
- [x] All main panels
- [x] Interactive features
- [x] Real AI provider integration
- [x] Command palette
- [x] Tests passing
- [x] Production-ready CLI integration

## Pain Points Addressed

This TUI directly addresses Forge's key pain points:

1. **Context (#1)**: Conversation panel shows streaming token-by-token output
2. **Verify (#4)**: Diff viewer integrates with verify-symbol for approve/reject
3. **Parallel (#5)**: Agent activity panel shows real-time parallel subagent status
4. **Resume (#9)**: Checkpoint banner enables one-key resume

## Rival Lessons Applied

- **Ink lag**: Uses ratatui (immediate-mode) instead of Ink (React-based) - stays responsive under load
- **Auggie diff**: Best-in-class diff viewer with per-hunk approve/reject/edit - opposite of "accept on faith"