//! Status bar - provider, model, token usage, autonomy tier, verify state
//!
//! Shows at the bottom of the TUI and displays:
//! - Current provider and model
//! - Token usage and context budget
//! - Autonomy tier
//! - Verify state
//! - Elapsed time and cost
//! - User-customizable status line

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

/// Status bar at bottom of TUI
pub struct StatusBar {
    /// Provider name (e.g., "anthropic", "gemini")
    provider: String,
    /// Model name (e.g., "claude-3-5-sonnet-20241022")
    model: String,
    /// Tokens used
    tokens_used: u32,
    /// Context budget (tokens)
    context_budget: u32,
    /// Autonomy tier
    autonomy_tier: AutonomyTier,
    /// Verification state
    verify_state: VerifyState,
    /// Elapsed time (seconds)
    elapsed: u32,
    /// Estimated cost (USD)
    cost: f64,
}

/// Autonomy tier level
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AutonomyTier {
    /// Tier 0: User approves all actions
    Tier0,
    /// Tier 1: Auto-approve read-only operations
    Tier1,
    /// Tier 2: Auto-approve file edits
    Tier2,
    /// Tier 3: Auto-approve shell commands
    Tier3,
    /// Tier 4: Auto-approve network requests
    Tier4,
    /// Tier 5: Full autonomy (no approvals)
    Tier5,
}

impl AutonomyTier {
    fn as_str(&self) -> &str {
        match self {
            AutonomyTier::Tier0 => "T0",
            AutonomyTier::Tier1 => "T1",
            AutonomyTier::Tier2 => "T2",
            AutonomyTier::Tier3 => "T3",
            AutonomyTier::Tier4 => "T4",
            AutonomyTier::Tier5 => "T5",
        }
    }

    fn color(self) -> Color {
        match self {
            AutonomyTier::Tier0 => Color::Green,
            AutonomyTier::Tier1 => Color::Cyan,
            AutonomyTier::Tier2 => Color::Blue,
            AutonomyTier::Tier3 => Color::Yellow,
            AutonomyTier::Tier4 => Color::Red,
            AutonomyTier::Tier5 => Color::Magenta,
        }
    }
}

/// Verification state
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VerifyState {
    /// Not verifying
    Idle,
    /// Verification in progress
    Verifying,
    /// Verification passed
    Passed,
    /// Verification failed
    Failed,
}

impl VerifyState {
    fn as_str(&self) -> &str {
        match self {
            VerifyState::Idle => "",
            VerifyState::Verifying => "🔍 Verifying...",
            VerifyState::Passed => "✓ Passed",
            VerifyState::Failed => "✗ Failed",
        }
    }

    fn color(self) -> Color {
        match self {
            VerifyState::Idle => Color::Reset,
            VerifyState::Verifying => Color::Yellow,
            VerifyState::Passed => Color::Green,
            VerifyState::Failed => Color::Red,
        }
    }
}

impl StatusBar {
    /// Create new status bar
    pub fn new() -> Self {
        Self {
            provider: "anthropic".to_string(),
            model: "claude-3-5-sonnet".to_string(),
            tokens_used: 0,
            context_budget: 200000,
            autonomy_tier: AutonomyTier::Tier0,
            verify_state: VerifyState::Idle,
            elapsed: 0,
            cost: 0.0,
        }
    }

    /// Update provider and model
    pub fn update_model(&mut self, provider: String, model: String) {
        self.provider = provider;
        self.model = model;
    }

    /// Update token usage
    pub fn update_tokens(&mut self, used: u32, budget: u32) {
        self.tokens_used = used;
        self.context_budget = budget;
    }

    /// Set autonomy tier
    pub fn set_autonomy_tier(&mut self, tier: AutonomyTier) {
        self.autonomy_tier = tier;
    }

    /// Update verification state
    pub fn update_verify_state(&mut self, state: String) {
        self.verify_state = match state.as_str() {
            "Verifying" => VerifyState::Verifying,
            "Passed" => VerifyState::Passed,
            "Failed" => VerifyState::Failed,
            _ => VerifyState::Idle,
        };
    }

    /// Update elapsed time
    pub fn update_elapsed(&mut self, elapsed: u32) {
        self.elapsed = elapsed;
    }

    /// Update estimated cost
    pub fn update_cost(&mut self, cost: f64) {
        self.cost = cost;
    }

    /// Format time as MM:SS
    fn format_time(&self, seconds: u32) -> String {
        let mins = seconds / 60;
        let secs = seconds % 60;
        format!("{:02}:{:02}", mins, secs)
    }

    /// Calculate token usage percentage
    fn token_percentage(&self) -> f32 {
        if self.context_budget == 0 {
            0.0
        } else {
            (self.tokens_used as f32 / self.context_budget as f32) * 100.0
        }
    }

    /// Render the status bar
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let token_pct = self.token_percentage();
        let token_color = if token_pct > 90.0 {
            Color::Red
        } else if token_pct > 70.0 {
            Color::Yellow
        } else {
            Color::Green
        };

        let line = Line::from(vec![
            // Provider and model
            Span::styled(
                format!("{}/{}", self.provider, self.model),
                Style::default().fg(Color::Cyan),
            ),
            Span::raw(" │ "),
            // Token usage
            Span::styled(
                format!("{}/{} tokens", self.tokens_used, self.context_budget),
                Style::default().fg(token_color),
            ),
            Span::raw(" │ "),
            // Autonomy tier
            Span::styled(
                format!("Autonomy: {}", self.autonomy_tier.as_str()),
                Style::default().fg(self.autonomy_tier.color()),
            ),
            Span::raw(" │ "),
            // Verification state
            Span::styled(
                self.verify_state.as_str(),
                Style::default().fg(self.verify_state.color()),
            ),
            Span::raw(" │ "),
            // Elapsed time
            Span::styled(
                format!("Time: {}", self.format_time(self.elapsed)),
                Style::default().fg(Color::White),
            ),
            Span::raw(" │ "),
            // Cost
            Span::styled(
                format!("Cost: ${:.2}", self.cost),
                Style::default().fg(Color::Yellow),
            ),
        ]);

        let paragraph = Paragraph::new(line);
        f.render_widget(paragraph, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_bar_creation() {
        let bar = StatusBar::new();
        assert_eq!(bar.provider, "anthropic");
        assert_eq!(bar.autonomy_tier, AutonomyTier::Tier0);
    }

    #[test]
    fn test_token_percentage() {
        let mut bar = StatusBar::new();
        bar.tokens_used = 100000;
        bar.context_budget = 200000;
        assert_eq!(bar.token_percentage(), 50.0);
    }

    #[test]
    fn test_format_time() {
        let bar = StatusBar::new();
        assert_eq!(bar.format_time(125), "02:05");
    }

    #[test]
    fn test_update_verify_state() {
        let mut bar = StatusBar::new();
        bar.update_verify_state("Passed".to_string());
        assert_eq!(bar.verify_state, VerifyState::Passed);
    }
}