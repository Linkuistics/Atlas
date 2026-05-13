//! Token panel widget. Renders the running token total; with
//! `show_providers` it also breaks the totals down by `Provider`.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use super::state::TuiState;
use atlas_llm::Provider;

/// Render the token panel.
///
/// `show_providers` is the CLI's `--tui-show-providers` flag — when
/// `true`, each provider gets its own line below the aggregate.
pub fn render(frame: &mut Frame, area: Rect, state: &TuiState, show_providers: bool) {
    let in_total = state.token_totals.total_in();
    let out_total = state.token_totals.total_out();

    let mut lines: Vec<Line> = vec![Line::from(vec![
        Span::styled("tokens ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!("in={in_total} out={out_total}")),
    ])];

    if show_providers {
        for (provider, totals) in &state.token_totals.by_provider {
            let label = match provider {
                Provider::Anthropic => "anthropic",
                Provider::OpenAi => "openai",
            };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(label.to_string(), Style::default().fg(Color::DarkGray)),
                Span::raw(format!(
                    " in={} out={}",
                    totals.tokens_in, totals.tokens_out
                )),
            ]));
        }
    }

    let block = Block::default().borders(Borders::ALL).title("Tokens");
    let para = Paragraph::new(lines).block(block);
    frame.render_widget(para, area);
}
