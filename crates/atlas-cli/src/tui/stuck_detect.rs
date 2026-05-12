//! Stuck-agent heuristic widget.
//!
//! Renders a "stuck on: <agent_id>" warning line when the
//! [`super::state::StuckDetector`] reports an elapsed-since-activity
//! window exceeding the 90s threshold. When no agent is stuck, the
//! widget renders a one-line "OK" status (the layout reserves two
//! lines for the bottom strip, so the field always has visible state).

use std::time::Instant;

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use super::state::{AgentStatus, TuiState};

/// Render the stuck-detector strip.
pub fn render(frame: &mut Frame, area: Rect, state: &TuiState) {
    let now = Instant::now();
    let line = if let Some(elapsed) = state.stuck.check(now) {
        // Pick the first agent still in Running state — that's the
        // best candidate for "stuck on".
        let stuck_agent = state
            .workspace_tree
            .by_stage
            .values()
            .flat_map(|bucket| bucket.iter())
            .find(|n| matches!(n.status, AgentStatus::Running))
            .map(|n| n.agent_id.clone())
            .unwrap_or_else(|| "<none>".into());
        Line::from(vec![
            Span::styled(
                "stuck on: ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw(stuck_agent),
            Span::raw(format!(" ({}s idle)", elapsed.as_secs())),
        ])
    } else {
        Line::from(vec![Span::styled("ok", Style::default().fg(Color::Green))])
    };
    let block = Block::default().borders(Borders::ALL).title("Health");
    let para = Paragraph::new(line).block(block);
    frame.render_widget(para, area);
}
