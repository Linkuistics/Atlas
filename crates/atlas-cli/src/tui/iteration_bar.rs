//! Iteration counter + convergence indicator.
//!
//! Convergence rule (plan §4 Task 6 Step 6.2):
//! - The two most recent `IterationBoundary` events agree on
//!   `prior_model_sha` ⇒ render the green check (`v`).
//! - Otherwise (single iteration so far, or two iterations whose
//!   priors differ) ⇒ render the yellow caret (`^`).
//!
//! PR-4's runtime emits a single `IterationBoundary` per run, so the
//! convergence indicator is dormant until PR-5 wires the fixedpoint
//! loop. PR-6's renderer handles the single-iteration case gracefully.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use super::state::TuiState;

/// Render the iteration bar into `area`.
pub fn render(frame: &mut Frame, area: Rect, state: &TuiState) {
    let (glyph, colour, label) = convergence_glyph(state);
    let line = Line::from(vec![
        Span::raw("iter "),
        Span::raw(format!("{}", state.iteration)),
        Span::raw("  "),
        Span::styled(glyph.to_string(), Style::default().fg(colour)),
        Span::raw(" "),
        Span::styled(label.to_string(), Style::default().fg(Color::DarkGray)),
    ]);
    let block = Block::default().borders(Borders::ALL).title("Iteration");
    let para = Paragraph::new(line).block(block);
    frame.render_widget(para, area);
}

fn convergence_glyph(state: &TuiState) -> (&'static str, Color, &'static str) {
    match (&state.prev_prior_model_sha, &state.last_prior_model_sha) {
        (Some(prev), Some(curr)) if prev == curr => ("v", Color::Green, "converged"),
        (Some(_), Some(_)) => ("^", Color::Yellow, "moving"),
        // Single iteration so far — PR-4 single-iteration regime, or
        // PR-5's first iteration. Render neutral.
        _ => ("-", Color::DarkGray, "single iteration"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convergence_glyph_single_iteration_is_neutral() {
        let s = TuiState::default();
        let (_, _, label) = convergence_glyph(&s);
        assert_eq!(label, "single iteration");
    }

    #[test]
    fn convergence_glyph_matching_priors_marks_converged() {
        let s = TuiState {
            last_prior_model_sha: Some("x".into()),
            prev_prior_model_sha: Some("x".into()),
            ..TuiState::default()
        };
        let (_, _, label) = convergence_glyph(&s);
        assert_eq!(label, "converged");
    }

    #[test]
    fn convergence_glyph_differing_priors_marks_moving() {
        let s = TuiState {
            last_prior_model_sha: Some("a".into()),
            prev_prior_model_sha: Some("b".into()),
            ..TuiState::default()
        };
        let (_, _, label) = convergence_glyph(&s);
        assert_eq!(label, "moving");
    }
}
