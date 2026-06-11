use crate::theme::Colors;
use crossterm::event::KeyCode;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

/// Renders the query input line.
pub struct EditorComponent {
    pub query: String,
    pub colors: Colors,
}

impl EditorComponent {
    /// Render the input line with prompt and cursor positioning.
    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        let prompt_text = format!("> {}", self.query);
        let prompt_style = Style::default().fg(self.colors.input_prompt);
        let input_paragraph = Paragraph::new(Span::raw(prompt_text)).style(prompt_style);
        frame.render_widget(input_paragraph, area);

        // Position cursor at end of typed query
        let cursor_x = 1 + (self.query.len() as u16);
        let max_x = area.x + area.width.saturating_sub(1);
        frame.set_cursor_position(Position {
            x: cursor_x.min(max_x),
            y: area.y,
        });
    }
}

/// Handle key events for the editor (query input).
pub fn handle_key(key: KeyCode, query: &mut String) -> Option<Action> {
    match key {
        KeyCode::Char('q') | KeyCode::Char('Q') => Some(Action::Quit),
        KeyCode::Char(c) => {
            query.push(c);
            None
        }
        KeyCode::Backspace => {
            query.pop();
            None
        }
        KeyCode::Enter => Some(Action::Submit),
        _ => None,
    }
}

pub enum Action {
    Quit,
    Submit,
}
