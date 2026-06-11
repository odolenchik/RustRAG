use ratatui::prelude::*;

/// Semantic colour tokens used throughout the TUI.
#[derive(Clone, Copy, Debug)]
pub struct Colors {
    pub title_fg: Color,
    pub title_bg: Color,
    pub input_prompt: Color,
    pub error_fg: Color,
    pub error_border: Color,
    pub loading_fg: Color,
    pub answer_fg: Color,
    pub answer_border: Color,
    pub highlight_bg: Color,
}

impl Colors {
    /// Dark palette — default for terminal use.
    pub fn dark() -> Self {
        Self {
            title_fg: Color::White,
            title_bg: Color::Blue,
            input_prompt: Color::Yellow,
            error_fg: Color::Red,
            error_border: Color::Red,
            loading_fg: Color::Green,
            answer_fg: Color::Green,
            answer_border: Color::Green,
            highlight_bg: Color::DarkGray,
        }
    }

    /// Light palette — for terminals with light backgrounds.
    pub fn light() -> Self {
        Self {
            title_fg: Color::White,
            title_bg: Color::Blue,
            input_prompt: Color::Cyan,
            error_fg: Color::Red,
            error_border: Color::Red,
            loading_fg: Color::DarkGray,
            answer_fg: Color::DarkGray,
            answer_border: Color::Green,
            highlight_bg: Color::Gray,
        }
    }

    /// Apply styled border and title area for an **error** block.
    pub fn error_block(&self) -> Style {
        Style::default().fg(self.error_fg).bg(Color::Reset)
    }

    /// Apply foreground style to a paragraph/line.
    pub fn fg(&self, color: Color) -> Style {
        Style::default().fg(color)
    }

    /// Highlight style for selected list items.
    pub fn highlight_style(&self) -> Style {
        Style::default().bg(self.highlight_bg)
    }
}

impl Default for Colors {
    fn default() -> Self {
        Self::dark()
    }
}
