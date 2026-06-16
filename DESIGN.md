---
name: Forge TUI Design System
description: A clean, high-density, keyboard-first terminal user interface for Forge.
colors:
  primary: "#00ffff"      # Cyan for primary actions and focused pane border
  neutral-bg: "#000000"   # Black for terminal background
  neutral-fg: "#ffffff"   # White for body text
  muted: "#808080"        # Gray for inactive borders and text
  success: "#00ff00"      # Green for assistant messages and passed states
  error: "#ff0000"        # Red for failed verifications
  warning: "#ffff00"      # Yellow for warnings and checkpoints
typography:
  display:
    fontFamily: "monospace"
    fontSize: "12px"
    fontWeight: "normal"
  body:
    fontFamily: "monospace"
    fontSize: "12px"
    fontWeight: "normal"
rounded:
  sm: "0px"
  md: "0px"
spacing:
  sm: "1ch"
  md: "2ch"
components:
  focused-pane:
    backgroundColor: "{colors.neutral-bg}"
    textColor: "{colors.neutral-fg}"
    rounded: "{rounded.sm}"
  unfocused-pane:
    backgroundColor: "{colors.neutral-bg}"
    textColor: "{colors.muted}"
    rounded: "{rounded.sm}"
---

# Design System: Forge TUI

## 1. Overview

**Creative North Star: "The Slate Terminal"**

Forge TUI is designed to be a high-density, professional terminal environment. It avoids decorative clutter and blinking elements, letting the developer's code and the agent's logic occupy the center of attention.

The layout consists of structured panels separating conversation history, active diff hunks, and parallel agent activities. Transitions between modes (e.g., Plan/Build) are indicated clearly with status indicators rather than visual decoration.

**Key Characteristics:**
- **High Information Density**: Compact spacing and text layouts that maximize readable code.
- **High Contrast Borders**: Focus states are communicated via border colors and styles.
- **Restrained Color Coding**: Accent colors are reserved strictly for semantic states (success, warning, error) and current focus.

## 2. Colors

Terminal colors map to standard ANSI and RGB colors. We prioritize high contrast to ensure readability across different terminal schemes.

### Primary
- **Focus Cyan** (#00ffff): Used to highlight active borders, inputs, and commands.

### Neutral
- **Slate Black** (#000000): Default terminal background.
- **Slate White** (#ffffff): Default text foreground.
- **Slate Gray** (#808080): Inactive borders, secondary details, and muted help text.

### Semantic
- **Success Green** (#00ff00): Used for assistant responses, passed verification tests, and approved diff hunks.
- **Warning Yellow** (#ffff00): Used for checkpoint banners, pending items in queues, and verifications in progress.
- **Error Red** (#ff0000): Used for error logs, failed verifications, and rejected diff hunks.

**The Focus Rule.** Only one panel may have the Focus Cyan border at any given time. Unfocused panels must use Slate Gray borders to guide the developer's attention immediately.

## 3. Typography

**Display Font:** Monospace system font
**Body Font:** Monospace system font

Since the TUI is rendered in a monospace terminal terminal cell grid, typography is defined by weight and casing modifications rather than font families.

### Hierarchy
- **Headline**: Bold, Uppercase, Cyan. Used for main titles and key warnings.
- **Body**: Regular, White. Used for assistant reasoning, log output, and normal text.
- **Label**: Bold, Slate Gray or Cyan. Used for field labels, status bar fields, and headers.
- **Code/Diff**: Regular. Syntactically highlighted or color-coded by diff operation (Green/Red).

## 4. Elevation

The TUI uses tonal layering and border character changes to establish layout depth rather than physical shadows.

**The Depth Rule.** Overlay dialogs (such as command menus or helper sheets) must render with a solid border and clear background fill that overwrites any underlying panel text, simulating a physical overlay on top of the main dashboard.

## 5. Components

### Panels
- **Active Pane**: Border styled with `Color::Cyan` or `Color::Blue`. Title label in Cyan.
- **Inactive Pane**: Border styled with `Color::DarkGray` or `Color::Reset`. Title label in Gray.

### Diff Viewer
- **Hunk Box**: Structured block showing old and new code.
- **Hunk Actions**: Visual markers (e.g., `[Enter] Approve`, `[r] Reject`) highlighted in Yellow.
- **Line Highlight**: Selected line in diff is highlighted with a background block color (Dark Gray) or terminal selection block.

### Status Bar
- **Bar Container**: Background colored dark gray (`Color::DarkGray`) or inverted.
- **Status Cells**: Separated by vertical bar characters (`│`). Active values are color-coded based on status (e.g. Green for idle/ready, Yellow for working).

## 6. Do's and Don'ts

### Do:
- **Do** use `Color::Cyan` to represent focus and active text entry.
- **Do** truncate long tool outputs to prevent flooding panels.
- **Do** display keyboard shortcuts in a dedicated helper row or panel.

### Don't:
- **Don't** use flashing or blinking styles for normal state displays.
- **Don't** use neon background fills that conflict with terminal themes.
- **Don't** overlap panel borders or draw double-width lines unless representing a modal dialog.
