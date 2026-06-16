# Product

## Register

product

## Users
Developers using the CLI locally in their terminal, editing/debugging code, and reviewing diff hunks. They are in a fast-paced development workflow and need clear, readable, and highly responsive feedback.

## Product Purpose
Forge TUI provides an interactive, terminal-native user interface for the Forge CLI coding agent. It enables developers to steer the agent, view streaming reasoning, inspect code changes hunk-by-hunk, and track parallel subagents.

## Brand Personality
Expert, efficient, precise, lightweight, and responsive.

## Anti-references
- Claude Code: Can feel cluttered or slow when context grows large, lack of modular pane division.
- Auggie: Primitive, non-interactive diff views that force developers to accept changes blindly.
- Over-decorated TUIs: Excessive use of blinking elements, arbitrary borders, or confusing color systems that distract from the code.

## Design Principles
1. **Tool-first transparency**: The UI should never distract from the developer's code or the agent's logic. Borders, labels, and status lines must guide focus, not consume it.
2. **Keyboard-driven speed**: Instant pane switching, command invocation, and diff navigation. Keyboard shortcuts should be intuitive and clear.
3. **Earned familiarity**: Use standard, clean terminal UI conventions (borders, status bars, clean separation of panels) that fit right into tmux/vim setups.

## Accessibility & Inclusion
- High-contrast terminal color mapping (respect user's terminal theme colors where possible).
- Support for standard screen sizes (80x24 minimum, scaled layouts for wider screens).
- No reliance on flashing/blinking colors to indicate crucial status.
