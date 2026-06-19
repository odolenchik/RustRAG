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

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // dark() palette
    // -----------------------------------------------------------------------

    #[test]
    fn test_dark_palette_title_fg_is_white() {
        let colors = Colors::dark();
        assert_eq!(colors.title_fg, Color::White);
    }

    #[test]
    fn test_dark_palette_title_bg_is_blue() {
        let colors = Colors::dark();
        assert_eq!(colors.title_bg, Color::Blue);
    }

    #[test]
    fn test_dark_palette_input_prompt_is_yellow() {
        let colors = Colors::dark();
        assert_eq!(colors.input_prompt, Color::Yellow);
    }

    #[test]
    fn test_dark_palette_error_fg_is_red() {
        let colors = Colors::dark();
        assert_eq!(colors.error_fg, Color::Red);
    }

    #[test]
    fn test_dark_palette_error_border_is_red() {
        let colors = Colors::dark();
        assert_eq!(colors.error_border, Color::Red);
    }

    #[test]
    fn test_dark_palette_loading_fg_is_green() {
        let colors = Colors::dark();
        assert_eq!(colors.loading_fg, Color::Green);
    }

    #[test]
    fn test_dark_palette_answer_fg_is_green() {
        let colors = Colors::dark();
        assert_eq!(colors.answer_fg, Color::Green);
    }

    #[test]
    fn test_dark_palette_answer_border_is_green() {
        let colors = Colors::dark();
        assert_eq!(colors.answer_border, Color::Green);
    }

    #[test]
    fn test_dark_palette_highlight_bg_is_dark_gray() {
        let colors = Colors::dark();
        assert_eq!(colors.highlight_bg, Color::DarkGray);
    }

    // -----------------------------------------------------------------------
    // light() palette
    // -----------------------------------------------------------------------

    #[test]
    fn test_light_palette_title_fg_is_white() {
        let colors = Colors::light();
        assert_eq!(colors.title_fg, Color::White);
    }

    #[test]
    fn test_light_palette_title_bg_is_blue() {
        let colors = Colors::light();
        assert_eq!(colors.title_bg, Color::Blue);
    }

    #[test]
    fn test_light_palette_input_prompt_is_cyan() {
        let colors = Colors::light();
        assert_eq!(colors.input_prompt, Color::Cyan);
    }

    #[test]
    fn test_light_palette_error_fg_is_red() {
        let colors = Colors::light();
        assert_eq!(colors.error_fg, Color::Red);
    }

    #[test]
    fn test_light_palette_error_border_is_red() {
        let colors = Colors::light();
        assert_eq!(colors.error_border, Color::Red);
    }

    #[test]
    fn test_light_palette_loading_fg_is_dark_gray() {
        let colors = Colors::light();
        assert_eq!(colors.loading_fg, Color::DarkGray);
    }

    #[test]
    fn test_light_palette_answer_fg_is_dark_gray() {
        let colors = Colors::light();
        assert_eq!(colors.answer_fg, Color::DarkGray);
    }

    #[test]
    fn test_light_palette_answer_border_is_green() {
        let colors = Colors::light();
        assert_eq!(colors.answer_border, Color::Green);
    }

    #[test]
    fn test_light_palette_highlight_bg_is_gray() {
        let colors = Colors::light();
        assert_eq!(colors.highlight_bg, Color::Gray);
    }

    // -----------------------------------------------------------------------
    // Default trait impl (should return dark palette)
    // -----------------------------------------------------------------------

    #[test]
    fn test_default_returns_dark_palette() {
        let default_colors: Colors = Default::default();
        let dark_colors = Colors::dark();

        assert_eq!(default_colors.title_fg, dark_colors.title_fg);
        assert_eq!(default_colors.title_bg, dark_colors.title_bg);
        assert_eq!(default_colors.input_prompt, dark_colors.input_prompt);
        assert_eq!(default_colors.error_fg, dark_colors.error_fg);
        assert_eq!(default_colors.error_border, dark_colors.error_border);
        assert_eq!(default_colors.loading_fg, dark_colors.loading_fg);
        assert_eq!(default_colors.answer_fg, dark_colors.answer_fg);
        assert_eq!(default_colors.answer_border, dark_colors.answer_border);
        assert_eq!(default_colors.highlight_bg, dark_colors.highlight_bg);
    }

    // -----------------------------------------------------------------------
    // error_block() method
    // -----------------------------------------------------------------------

    #[test]
    fn test_error_block_applies_error_fg() {
        let colors = Colors::dark();
        let style = colors.error_block();

        // The returned Style should have foreground set to error_fg (Red for dark)
        assert_eq!(style.fg, Some(Color::Red));
    }

    #[test]
    fn test_error_block_sets_background_to_reset() {
        let colors = Colors::dark();
        let style = colors.error_block();

        // Background should be Color::Reset
        assert_eq!(style.bg, Some(Color::Reset));
    }

    #[test]
    fn test_error_block_light_uses_red_for_fg() {
        let colors = Colors::light();
        let style = colors.error_block();

        assert_eq!(style.fg, Some(Color::Red));
    }

    // -----------------------------------------------------------------------
    // fg() method
    // -----------------------------------------------------------------------

    #[test]
    fn test_fg_applies_given_color_as_foreground() {
        let colors = Colors::dark();
        let style = colors.fg(Color::Magenta);

        assert_eq!(style.fg, Some(Color::Magenta));
    }

    #[test]
    fn test_fg_multiple_colors() {
        let colors = Colors::light();

        let white_style = colors.fg(Color::White);
        let cyan_style = colors.fg(Color::Cyan);
        let red_style = colors.fg(Color::Red);

        assert_eq!(white_style.fg, Some(Color::White));
        assert_eq!(cyan_style.fg, Some(Color::Cyan));
        assert_eq!(red_style.fg, Some(Color::Red));
    }

    // -----------------------------------------------------------------------
    // highlight_style() method
    // -----------------------------------------------------------------------

    #[test]
    fn test_highlight_style_applies_dark_bg_for_dark_palette() {
        let colors = Colors::dark();
        let style = colors.highlight_style();

        assert_eq!(style.bg, Some(Color::DarkGray));
    }

    #[test]
    fn test_highlight_style_applies_gray_bg_for_light_palette() {
        let colors = Colors::light();
        let style = colors.highlight_style();

        assert_eq!(style.bg, Some(Color::Gray));
    }

    // -----------------------------------------------------------------------
    // Clone and Copy traits
    // -----------------------------------------------------------------------

    #[test]
    fn test_colors_clone_and_copy() {
        let colors = Colors::dark();
        let cloned = colors; // copy
        let copied = colors.clone();

        assert_eq!(cloned.title_fg, copied.title_fg);
        assert_eq!(cloned.input_prompt, copied.input_prompt);
        assert_eq!(cloned.highlight_bg, copied.highlight_bg);
    }

    #[test]
    fn test_colors_debug() {
        let colors = Colors::dark();
        let debug_str = format!("{:?}", colors);
        // Should contain recognizable tokens
        assert!(debug_str.contains("Colors"));
    }

    // -----------------------------------------------------------------------
    // dark vs light palette differences
    // -----------------------------------------------------------------------

    #[test]
    fn test_dark_and_light_palette_differ() {
        let dark = Colors::dark();
        let light = Colors::light();

        // input_prompt: Yellow (dark) != Cyan (light)
        assert_ne!(dark.input_prompt, light.input_prompt);

        // loading_fg: Green (dark) != DarkGray (light)
        assert_ne!(dark.loading_fg, light.loading_fg);

        // answer_fg: Green (dark) != DarkGray (light)
        assert_ne!(dark.answer_fg, light.answer_fg);

        // highlight_bg: DarkGray (dark) != Gray (light)
        assert_ne!(dark.highlight_bg, light.highlight_bg);

        // title and error colors should be the same
        assert_eq!(dark.title_fg, light.title_fg);
        assert_eq!(dark.title_bg, light.title_bg);
        assert_eq!(dark.error_fg, light.error_fg);
        assert_eq!(dark.error_border, light.error_border);
    }

    // -----------------------------------------------------------------------
    // error_block returns a valid Style with no side effects
    // -----------------------------------------------------------------------

    #[test]
    fn test_error_block_style_does_not_panic() {
        let dark = Colors::dark();
        let style = dark.error_block();
        assert!(style.fg.is_some());
        assert!(style.bg.is_some());
    }

    #[test]
    fn test_fg_returns_valid_style() {
        let colors = Colors::light();
        let style = colors.fg(Color::Black);
        assert!(style.fg.is_some());
    }

    // -----------------------------------------------------------------------
    // highlight_style returns valid Style with correct background
    // -----------------------------------------------------------------------

    #[test]
    fn test_highlight_style_returns_valid_style() {
        let dark = Colors::dark();
        let style = dark.highlight_style();
        assert!(style.bg.is_some());
    }
}
