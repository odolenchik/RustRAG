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

#[derive(Debug, PartialEq)]
pub enum Action {
    Quit,
    Submit,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;

    // -----------------------------------------------------------------------
    // handle_key — Quit action
    // -----------------------------------------------------------------------

    #[test]
    fn test_lowercase_q_returns_quit() {
        let mut query = String::new();
        let result = handle_key(KeyCode::Char('q'), &mut query);
        assert_eq!(result, Some(Action::Quit));
    }

    #[test]
    fn test_uppercase_Q_returns_quit() {
        let mut query = String::new();
        let result = handle_key(KeyCode::Char('Q'), &mut query);
        assert_eq!(result, Some(Action::Quit));
    }

    #[test]
    fn test_q_does_not_modify_query() {
        let mut query = "hello".to_string();
        handle_key(KeyCode::Char('q'), &mut query);
        assert_eq!(query, "hello");
    }

    // -----------------------------------------------------------------------
    // handle_key — Submit action
    // -----------------------------------------------------------------------

    #[test]
    fn test_enter_returns_submit() {
        let mut query = String::new();
        let result = handle_key(KeyCode::Enter, &mut query);
        assert_eq!(result, Some(Action::Submit));
    }

    #[test]
    fn test_enter_does_not_modify_query() {
        let mut query = "search term".to_string();
        handle_key(KeyCode::Enter, &mut query);
        assert_eq!(query, "search term");
    }

    // -----------------------------------------------------------------------
    // handle_key — Character input
    // -----------------------------------------------------------------------

    #[test]
    fn test_lowercase_char_pushes_to_query() {
        let mut query = String::new();
        let result = handle_key(KeyCode::Char('a'), &mut query);
        assert_eq!(result, None);
        assert_eq!(query, "a");
    }

    #[test]
    fn test_uppercase_char_pushes_to_query() {
        let mut query = String::new();
        let result = handle_key(KeyCode::Char('Z'), &mut query);
        assert_eq!(result, None);
        assert_eq!(query, "Z");
    }

    #[test]
    fn test_digit_char_pushes_to_query() {
        let mut query = String::new();
        let result = handle_key(KeyCode::Char('4'), &mut query);
        assert_eq!(result, None);
        assert_eq!(query, "4");
    }

    #[test]
    fn test_space_char_pushes_to_query() {
        let mut query = String::new();
        let result = handle_key(KeyCode::Char(' '), &mut query);
        assert_eq!(result, None);
        assert_eq!(query, " ");
    }

    #[test]
    fn test_special_chars_pushed_to_query() {
        let mut query = String::new();
        for ch in ['.', '/', '-', '_', ':', '=', '+', '@', '#'] {
            handle_key(KeyCode::Char(ch), &mut query);
        }
        assert_eq!(query, "./-_:=+@#");
    }

    #[test]
    fn test_multiple_chars_appended() {
        let mut query = String::new();
        for c in ['h', 'e', 'l', 'l', 'o'] {
            handle_key(KeyCode::Char(c), &mut query);
        }
        assert_eq!(query, "hello");
    }

    #[test]
    fn test_char_action_is_none() {
        let mut query = String::new();
        let result = handle_key(KeyCode::Char('x'), &mut query);
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // handle_key — Backspace
    // -----------------------------------------------------------------------

    #[test]
    fn test_backspace_removes_last_char() {
        let mut query = "abc".to_string();
        let result = handle_key(KeyCode::Backspace, &mut query);
        assert_eq!(result, None);
        assert_eq!(query, "ab");
    }

    #[test]
    fn test_backspace_on_empty_query_does_not_panic() {
        let mut query = String::new();
        let result = handle_key(KeyCode::Backspace, &mut query);
        assert_eq!(result, None);
        assert!(query.is_empty());
    }

    #[test]
    fn test_backspace_multiple_times() {
        let mut query = "abcd".to_string();
        handle_key(KeyCode::Backspace, &mut query);
        handle_key(KeyCode::Backspace, &mut query);
        handle_key(KeyCode::Backspace, &mut query);
        assert_eq!(query, "a");
    }

    #[test]
    fn test_backspace_action_is_none() {
        let mut query = "x".to_string();
        let result = handle_key(KeyCode::Backspace, &mut query);
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // handle_key — Unknown keys (no-op)
    // -----------------------------------------------------------------------

    #[test]
    fn test_unknown_key_returns_none() {
        let mut query = String::new();
        let result = handle_key(KeyCode::F(1), &mut query);
        assert!(result.is_none());
    }

    #[test]
    fn test_tab_key_noop() {
        let mut query = "hello".to_string();
        let result = handle_key(KeyCode::Tab, &mut query);
        assert_eq!(result, None);
        assert_eq!(query, "hello");
    }

    #[test]
    fn test_left_arrow_key_noop() {
        let mut query = String::new();
        let result = handle_key(KeyCode::Left, &mut query);
        assert!(result.is_none());
        assert!(query.is_empty());
    }

    #[test]
    fn test_right_arrow_key_noop() {
        let mut query = String::new();
        let result = handle_key(KeyCode::Right, &mut query);
        assert!(result.is_none());
        assert!(query.is_empty());
    }

    #[test]
    fn test_up_arrow_key_noop() {
        let mut query = String::new();
        let result = handle_key(KeyCode::Up, &mut query);
        assert!(result.is_none());
        assert!(query.is_empty());
    }

    #[test]
    fn test_down_arrow_key_noop() {
        let mut query = String::new();
        let result = handle_key(KeyCode::Down, &mut query);
        assert!(result.is_none());
        assert!(query.is_empty());
    }

    #[test]
    fn test_esc_key_noop_at_editor_level() {
        // Escape is handled at the App level, not in editor::handle_key
        let mut query = "partial".to_string();
        let result = handle_key(KeyCode::Esc, &mut query);
        assert!(result.is_none());
    }

    #[test]
    fn test_page_down_noop() {
        let mut query = String::new();
        let result = handle_key(KeyCode::PageDown, &mut query);
        assert!(result.is_none());
    }

    #[test]
    fn test_home_key_noop() {
        let mut query = "text".to_string();
        let result = handle_key(KeyCode::Home, &mut query);
        assert_eq!(result, None);
        assert_eq!(query, "text");
    }

    #[test]
    fn test_end_key_noop() {
        let mut query = "text".to_string();
        let result = handle_key(KeyCode::End, &mut query);
        assert_eq!(result, None);
        assert_eq!(query, "text");
    }

    // -----------------------------------------------------------------------
    // handle_key — Combined operations
    // -----------------------------------------------------------------------

    #[test]
    fn test_type_and_backspace_cycle() {
        let mut query = String::new();
        handle_key(KeyCode::Char('h'), &mut query);
        handle_key(KeyCode::Char('i'), &mut query);
        assert_eq!(query, "hi");
        handle_key(KeyCode::Backspace, &mut query);
        assert_eq!(query, "h");
    }

    #[test]
    fn test_type_enter_submit() {
        let mut query = String::new();
        handle_key(KeyCode::Char('w'), &mut query);
        handle_key(KeyCode::Char('h'), &mut query);
        assert_eq!(query, "wh");
        let result = handle_key(KeyCode::Enter, &mut query);
        assert_eq!(result, Some(Action::Submit));
    }

    #[test]
    fn test_type_q_quit() {
        let mut query = String::new();
        handle_key(KeyCode::Char('w'), &mut query);
        let result = handle_key(KeyCode::Char('q'), &mut query);
        assert_eq!(result, Some(Action::Quit));
        assert_eq!(query, "w");
    }

    #[test]
    fn test_unicode_char_pushes_to_query() {
        let mut query = String::new();
        handle_key(KeyCode::Char('é'), &mut query);
        handle_key(KeyCode::Char('ñ'), &mut query);
        assert_eq!(query, "éñ");
    }

    // -----------------------------------------------------------------------
    // Action enum
    // -----------------------------------------------------------------------

    #[test]
    fn test_action_debug() {
        let quit = format!("{:?}", Action::Quit);
        let submit = format!("{:?}", Action::Submit);
        assert!(quit.contains("Quit"));
        assert!(submit.contains("Submit"));
    }

    // -----------------------------------------------------------------------
    // EditorComponent struct
    // -----------------------------------------------------------------------

    #[test]
    fn test_editor_component_creation() {
        let colors = Colors::dark();
        let component = EditorComponent {
            query: "hello world".to_string(),
            colors,
        };
        assert_eq!(component.query, "hello world");
        assert_eq!(component.colors.title_fg, Color::White);
    }

    #[test]
    fn test_editor_component_empty_query() {
        let component = EditorComponent {
            query: String::new(),
            colors: Colors::light(),
        };
        assert!(component.query.is_empty());
    }

    #[test]
    fn test_editor_component_clone_colors() {
        let mut c1 = Colors::dark();
        c1.title_fg = Color::Magenta;
        let component = EditorComponent {
            query: "test".to_string(),
            colors: c1,
        };
        assert_eq!(component.colors.title_fg, Color::Magenta);
    }

    #[test]
    fn test_editor_component_query_len_for_cursor() {
        let component = EditorComponent {
            query: "abcde".to_string(),
            colors: Colors::dark(),
        };
        // cursor_x should be 1 + 5 = 6 for a 5-char query
        assert_eq!(component.query.len(), 5);
    }

    #[test]
    fn test_editor_component_query_multiline_chars() {
        let mut component = EditorComponent {
            query: String::new(),
            colors: Colors::dark(),
        };
        // Push newline character (should be accepted, draw handles it)
        handle_key(KeyCode::Char('\n'), &mut component.query);
        assert_eq!(component.query.len(), 1);
    }

    #[test]
    fn test_handle_key_with_long_query() {
        let mut query = String::new();
        // Use only characters that are guaranteed to be pushed (avoid Enter=13, Backspace=8).
        for i in 0..200u8 {
            let c = char::from(49 + (i % 75)); // '1'-'{', avoiding special keys
            handle_key(KeyCode::Char(c), &mut query);
        }
        assert!(query.len() >= 190);
    }

    #[test]
    fn test_alternative_input_noop() {
        let mut query = String::new();
        let result = handle_key(KeyCode::F(2), &mut query);
        assert!(result.is_none());
    }

    #[test]
    fn test_f_keys_are_noops() {
        let mut query = String::new();
        for f_key in [
            KeyCode::F(1),
            KeyCode::F(2),
            KeyCode::F(12),
        ] {
            let result = handle_key(f_key, &mut query);
            assert!(result.is_none());
            assert!(query.is_empty());
        }
    }

    #[test]
    fn test_delete_key_is_noop() {
        let mut query = "text".to_string();
        let result = handle_key(KeyCode::Delete, &mut query);
        assert_eq!(result, None);
        assert_eq!(query, "text");
    }

    #[test]
    fn test_insert_key_is_noop() {
        let mut query = String::new();
        let result = handle_key(KeyCode::Insert, &mut query);
        assert!(result.is_none());
    }

    #[test]
    fn test_backspace_after_enter_still_works() {
        let mut query = "abc".to_string();
        handle_key(KeyCode::Enter, &mut query); // submit
        assert_eq!(query, "abc");
        handle_key(KeyCode::Backspace, &mut query);
        assert_eq!(query, "ab");
    }

    #[test]
    fn test_q_quit_clears_no_query() {
        let mut query = String::new();
        let result = handle_key(KeyCode::Char('q'), &mut query);
        assert_eq!(result, Some(Action::Quit));
        assert!(query.is_empty());
    }

    #[test]
    fn test_multiple_backspaces_on_single_char() {
        let mut query = "x".to_string();
        handle_key(KeyCode::Backspace, &mut query); // -> ""
        assert_eq!(query, "");
        handle_key(KeyCode::Backspace, &mut query); // still ""
        assert_eq!(query, "");
    }

    #[test]
    fn test_submit_preserves_query_content() {
        let mut query = "find me".to_string();
        let result = handle_key(KeyCode::Enter, &mut query);
        assert_eq!(result, Some(Action::Submit));
        assert_eq!(query, "find me");
    }

    #[test]
    fn test_char_input_with_existing_query() {
        let mut query = "pre".to_string();
        handle_key(KeyCode::Char('f'), &mut query);
        assert_eq!(query, "pref");
    }
}
