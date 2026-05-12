//! Workspace-tree widget. Renders the per-stage agent list with each
//! agent's status as a glyph + colour.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use super::state::{AgentNode, AgentStatus, TuiState};
use atlas_agents::events::CacheHitSource;
use atlas_agents::Grade;

/// Render the workspace tree into `area`.
pub fn render(frame: &mut Frame, area: Rect, state: &TuiState) {
    let mut lines: Vec<Line> = Vec::new();
    if state.workspace_tree.by_stage.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (no agents yet)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for (stage, agents) in &state.workspace_tree.by_stage {
            lines.push(Line::from(Span::styled(
                stage.to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
            for agent in agents {
                lines.push(render_agent_line(agent));
            }
        }
    }
    let block = Block::default().borders(Borders::ALL).title("Workspace");
    let para = Paragraph::new(lines).block(block);
    frame.render_widget(para, area);
}

fn render_agent_line(agent: &AgentNode) -> Line<'static> {
    let (glyph, colour, status_text) = status_glyph(&agent.status);
    let target = agent.target.clone();
    let agent_id = agent.agent_id.clone();
    Line::from(vec![
        Span::raw("  "),
        Span::styled(glyph.to_string(), Style::default().fg(colour)),
        Span::raw(" "),
        Span::raw(agent_id),
        Span::raw(" "),
        Span::styled(
            format!("[{}]", target),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw(" "),
        Span::styled(status_text, Style::default().fg(Color::DarkGray)),
    ])
}

fn status_glyph(status: &AgentStatus) -> (&'static str, Color, String) {
    match status {
        AgentStatus::Running => ("*", Color::Yellow, "running".into()),
        AgentStatus::Complete { grade } => {
            let suffix = match grade {
                Grade::Strong => "strong",
                Grade::Moderate => "moderate",
                Grade::Weak => "weak",
                Grade::Declines => "declines",
            };
            ("v", Color::Green, format!("done ({suffix})"))
        }
        AgentStatus::HardFailed { error_kind } => ("x", Color::Red, format!("fail ({error_kind})")),
        AgentStatus::CacheHit { source } => {
            let label = match source {
                CacheHitSource::AgentCache => "cache",
                CacheHitSource::DispatchedFromOverride => "override",
            };
            ("o", Color::Blue, format!("hit ({label})"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_glyph_covers_all_variants() {
        let _ = status_glyph(&AgentStatus::Running);
        let _ = status_glyph(&AgentStatus::Complete {
            grade: Grade::Strong,
        });
        let _ = status_glyph(&AgentStatus::HardFailed {
            error_kind: "x".into(),
        });
        let _ = status_glyph(&AgentStatus::CacheHit {
            source: CacheHitSource::AgentCache,
        });
    }
}
