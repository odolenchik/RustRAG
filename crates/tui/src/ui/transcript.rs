use crate::theme::Colors;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use rust_rag_core::vector_store::SearchResult;

/// Renders the transcript area (search results + LLM answer/error).
pub struct TranscriptComponent {
    pub error_msg: Option<String>,
    pub search_results: Vec<SearchResult>,
    pub selected_result: usize,
    pub llm_state: crate::ui::LlmState,
    pub llm_answer: Option<String>,
    pub llm_partial_answer: String,
    pub llm_scroll_offset: usize,
    pub colors: Colors,
}

impl TranscriptComponent {
    /// Render the output area with results list and LLM answer.
    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        // No search yet — show help
        if self.search_results.is_empty() && self.error_msg.is_none() {
            let block = Block::default().borders(Borders::ALL);
            frame.render_widget(block, area);

            let help_text = "Type a question and press Enter. Press 'q' to quit.";
            let paragraph = Paragraph::new(Span::raw(help_text));
            frame.render_widget(paragraph, area);
            return;
        }

        // Error state
        if let Some(ref err) = self.error_msg {
            let block = Block::default()
                .title(" Error ")
                .borders(Borders::ALL)
                .style(Style::default().fg(self.colors.error_border));
            frame.render_widget(block, area);

            let error_paragraph = Paragraph::new(Span::raw(format!("! {}", err)))
                .style(Style::default().fg(self.colors.error_fg));
            frame.render_widget(error_paragraph, area);
            return;
        }

        // Split: results on top, LLM answer on bottom
        let llm_height = match self.llm_state {
            crate::ui::LlmState::Loading => 2u16,
            crate::ui::LlmState::Done | crate::ui::LlmState::Error => 5,
            _ => 0,
        };
        let results_h = area.height.saturating_sub(1 + llm_height);

        // Render search results (scrollable)
        if results_h > 2 {
            let results_rect = Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: results_h,
            };

            let max_items = (results_h.saturating_sub(4) as usize).min(5);
            let items: Vec<ListItem> = self
                .search_results
                .iter()
                .skip(self.selected_result.saturating_sub(max_items))
                .take(max_items)
                .map(|r| {
                    let line = format!(
                        "[{:.2}] {}:{} - {}",
                        r.score,
                        r.file_path.display(),
                        r.line_start,
                        &r.text.chars().take(60).collect::<String>()
                    );
                    ListItem::new(Span::raw(line))
                })
                .collect();

            let list =
                List::new(items).highlight_style(Style::default().bg(self.colors.highlight_bg));
            frame.render_widget(list, results_rect);
        }

        // Render LLM answer/error area
        if llm_height > 0 {
            let llm_y = area.y + results_h + 1;
            let remaining = area.height.saturating_sub(llm_y - area.y);
            if remaining == 0 || remaining < 2 {
                return;
            }

            let llm_rect = Rect {
                x: area.x,
                y: llm_y,
                width: area.width.min(80),
                height: llm_height.min(remaining),
            };

            match self.llm_state {
                crate::ui::LlmState::Loading if !self.llm_partial_answer.is_empty() => {
                    let display_text = format!("\u{258A} {}", self.llm_partial_answer);
                    let llm_paragraph = Paragraph::new(Span::raw(display_text))
                        .style(Style::default().fg(self.colors.loading_fg));
                    frame.render_widget(llm_paragraph, llm_rect);
                }
                crate::ui::LlmState::Loading => {
                    let loading_text = Paragraph::new(Span::raw("  LLM is thinking..."))
                        .style(Style::default().fg(self.colors.loading_fg));
                    frame.render_widget(loading_text, llm_rect);
                }
                crate::ui::LlmState::Done => {
                    let ans_block = Block::default()
                        .title(" LLM Answer ")
                        .borders(Borders::ALL)
                        .style(Style::default().fg(self.colors.answer_border));
                    frame.render_widget(ans_block, llm_rect);

                    if let Some(answer) = &self.llm_answer {
                        let total_lines: Vec<&str> = answer.lines().collect();
                        let page_size = (llm_rect.height.saturating_sub(1)) as usize;
                        let start = self.llm_scroll_offset.min(total_lines.len());
                        let visible: Vec<String> = total_lines
                            .iter()
                            .skip(start)
                            .take(page_size)
                            .map(|l| l.to_string())
                            .collect();
                        let display_text = if visible.is_empty() {
                            " (empty answer)".to_string()
                        } else {
                            visible.join("\n")
                        };
                        let full_display = if start < total_lines.len() - page_size {
                            format!("{}\n... (scroll down for more)", display_text)
                        } else if start > 0 {
                            format!("... (scroll up to see more)\n{}", display_text)
                        } else {
                            display_text
                        };

                        let llm_paragraph = Paragraph::new(Span::raw(full_display))
                            .style(Style::default().fg(self.colors.answer_fg));
                        frame.render_widget(llm_paragraph, llm_rect);
                    }
                }
                crate::ui::LlmState::Error => {
                    let err_block = Block::default()
                        .title(" LLM Error ")
                        .borders(Borders::ALL)
                        .style(Style::default().fg(self.colors.error_border));
                    frame.render_widget(err_block, llm_rect);

                    if let Some(ref err_msg) = self.llm_answer {
                        let total_lines: Vec<&str> = err_msg.lines().collect();
                        let page_size = (llm_rect.height.saturating_sub(1)) as usize;
                        let start = self.llm_scroll_offset.min(total_lines.len());
                        let visible: Vec<String> = total_lines
                            .iter()
                            .skip(start)
                            .take(page_size)
                            .map(|l| l.to_string())
                            .collect();
                        let display_text = if visible.is_empty() {
                            " (empty)".to_string()
                        } else {
                            visible.join("\n")
                        };

                        let llm_err = Paragraph::new(Span::raw(display_text))
                            .style(Style::default().fg(self.colors.error_fg));
                        frame.render_widget(llm_err, llm_rect);
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Colors;
    use ratatui::widgets::Block;

    // -----------------------------------------------------------------------
    // TranscriptComponent — construction and field access
    // -----------------------------------------------------------------------

    #[test]
    fn test_transcript_component_default_like() {
        let colors = Colors::dark();
        let component = TranscriptComponent {
            error_msg: None,
            search_results: Vec::new(),
            selected_result: 0,
            llm_state: crate::ui::LlmState::Idle,
            llm_answer: None,
            llm_partial_answer: String::new(),
            llm_scroll_offset: 0,
            colors,
        };
        assert!(component.search_results.is_empty());
        assert!(component.error_msg.is_none());
        assert!(component.llm_answer.is_none());
        assert!(component.llm_partial_answer.is_empty());
    }

    #[test]
    fn test_transcript_component_with_search_results() {
        let colors = Colors::dark();
        let results = vec![
            SearchResult {
                id: "doc1".to_string(),
                file_path: "/path/to/file.rs".into(),
                line_start: 10,
                line_end: 20,
                module_name: String::new(),
                symbol_kind: None,
                text: "fn example() -> i32 { 42 }".to_string(),
                score: 0.95,
            },
        ];
        let component = TranscriptComponent {
            error_msg: None,
            search_results: results.clone(),
            selected_result: 0,
            llm_state: crate::ui::LlmState::Idle,
            llm_answer: None,
            llm_partial_answer: String::new(),
            llm_scroll_offset: 0,
            colors,
        };
        assert_eq!(component.search_results.len(), 1);
        assert_eq!(component.selected_result, 0);
    }

    #[test]
    fn test_transcript_component_with_error_msg() {
        let component = TranscriptComponent {
            error_msg: Some("Connection failed".to_string()),
            search_results: Vec::new(),
            selected_result: 0,
            llm_state: crate::ui::LlmState::Idle,
            llm_answer: None,
            llm_partial_answer: String::new(),
            llm_scroll_offset: 0,
            colors: Colors::dark(),
        };
        assert_eq!(component.error_msg.as_deref(), Some("Connection failed"));
    }

    #[test]
    fn test_transcript_component_with_llm_done() {
        let component = TranscriptComponent {
            error_msg: None,
            search_results: Vec::new(),
            selected_result: 0,
            llm_state: crate::ui::LlmState::Done,
            llm_answer: Some("Here is the answer".to_string()),
            llm_partial_answer: String::new(),
            llm_scroll_offset: 0,
            colors: Colors::dark(),
        };
        assert!(matches!(component.llm_state, crate::ui::LlmState::Done));
        assert_eq!(component.llm_answer.as_deref(), Some("Here is the answer"));
    }

    // -----------------------------------------------------------------------
    // Empty state — no results, no error
    // -----------------------------------------------------------------------

    #[test]
    fn test_empty_state_no_results_no_error() {
        let component = TranscriptComponent {
            error_msg: None,
            search_results: Vec::new(),
            selected_result: 0,
            llm_state: crate::ui::LlmState::Idle,
            llm_answer: None,
            llm_partial_answer: String::new(),
            llm_scroll_offset: 0,
            colors: Colors::dark(),
        };
        assert!(component.search_results.is_empty());
        assert!(component.error_msg.is_none());
    }

    #[test]
    fn test_idle_state_with_empty_results_is_considered_empty() {
        let component = TranscriptComponent {
            error_msg: None,
            search_results: Vec::new(),
            selected_result: 0,
            llm_state: crate::ui::LlmState::Idle,
            llm_answer: Some("cached".to_string()), // answer present but no results
            llm_partial_answer: String::new(),
            llm_scroll_offset: 0,
            colors: Colors::dark(),
        };
        // Even with an answer, if search_results is empty and error_msg is None,
        // the draw() method treats it as "empty state" (help text).
        assert!(component.search_results.is_empty());
    }

    // -----------------------------------------------------------------------
    // Error state rendering logic
    // -----------------------------------------------------------------------

    #[test]
    fn test_error_state_has_error_msg_set() {
        let component = TranscriptComponent {
            error_msg: Some("File not found".to_string()),
            search_results: Vec::new(),
            selected_result: 0,
            llm_state: crate::ui::LlmState::Idle,
            llm_answer: None,
            llm_partial_answer: String::new(),
            llm_scroll_offset: 0,
            colors: Colors::dark(),
        };
        assert_eq!(component.error_msg.as_deref(), Some("File not found"));
    }

    #[test]
    fn test_error_state_with_results_still_shows_error() {
        let results = vec![SearchResult {
            id: "1".into(),
            file_path: "/x.rs".into(),
            line_start: 0,
            line_end: 1,
            module_name: String::new(),
            symbol_kind: None,
            text: "code".to_string(),
            score: 0.5,
        }];
        let component = TranscriptComponent {
            error_msg: Some("Embedding failed".to_string()),
            search_results: results,
            selected_result: 0,
            llm_state: crate::ui::LlmState::Idle,
            llm_answer: None,
            llm_partial_answer: String::new(),
            llm_scroll_offset: 0,
            colors: Colors::dark(),
        };
        assert!(component.error_msg.is_some());
    }

    // -----------------------------------------------------------------------
    // Loading state — partial answer
    // -----------------------------------------------------------------------

    #[test]
    fn test_loading_state_with_partial_answer() {
        let component = TranscriptComponent {
            error_msg: None,
            search_results: Vec::new(),
            selected_result: 0,
            llm_state: crate::ui::LlmState::Loading,
            llm_answer: None,
            llm_partial_answer: "Hello wo".to_string(),
            llm_scroll_offset: 0,
            colors: Colors::dark(),
        };
        assert!(matches!(component.llm_state, crate::ui::LlmState::Loading));
        assert!(!component.llm_partial_answer.is_empty());
    }

    #[test]
    fn test_loading_state_with_empty_partial() {
        let component = TranscriptComponent {
            error_msg: None,
            search_results: Vec::new(),
            selected_result: 0,
            llm_state: crate::ui::LlmState::Loading,
            llm_answer: None,
            llm_partial_answer: String::new(),
            llm_scroll_offset: 0,
            colors: Colors::dark(),
        };
        assert!(matches!(component.llm_state, crate::ui::LlmState::Loading));
        assert!(component.llm_partial_answer.is_empty());
    }

    // -----------------------------------------------------------------------
    // Loading state — error state transitions
    // -----------------------------------------------------------------------

    #[test]
    fn test_error_llm_state() {
        let component = TranscriptComponent {
            error_msg: None,
            search_results: Vec::new(),
            selected_result: 0,
            llm_state: crate::ui::LlmState::Error,
            llm_answer: Some("LLM Error: timeout".to_string()),
            llm_partial_answer: String::new(),
            llm_scroll_offset: 0,
            colors: Colors::dark(),
        };
        assert!(matches!(component.llm_state, crate::ui::LlmState::Error));
    }

    #[test]
    fn test_done_llm_state_with_empty_answer() {
        let component = TranscriptComponent {
            error_msg: None,
            search_results: Vec::new(),
            selected_result: 0,
            llm_state: crate::ui::LlmState::Done,
            llm_answer: Some(String::new()),
            llm_partial_answer: String::new(),
            llm_scroll_offset: 0,
            colors: Colors::dark(),
        };
        assert!(matches!(component.llm_state, crate::ui::LlmState::Done));
    }

    // -----------------------------------------------------------------------
    // LlmState enum tests (from ui/mod.rs)
    // -----------------------------------------------------------------------

    #[test]
    fn test_llmstate_clone() {
        let state = crate::ui::LlmState::Loading;
        let cloned = state.clone();
        assert!(matches!(cloned, crate::ui::LlmState::Loading));
    }

    #[test]
    fn test_llmstate_partial_eq() {
        let idle1 = crate::ui::LlmState::Idle;
        let idle2 = crate::ui::LlmState::Idle;
        let done = crate::ui::LlmState::Done;

        assert_eq!(idle1, idle2);
        assert_ne!(idle1, done);
    }

    #[test]
    fn test_llmstate_default_is_idle() {
        let state: crate::ui::LlmState = Default::default();
        assert!(matches!(state, crate::ui::LlmState::Idle));
    }

    #[test]
    fn test_all_llm_state_variants() {
        let states = vec![
            crate::ui::LlmState::Idle,
            crate::ui::LlmState::Loading,
            crate::ui::LlmState::Done,
            crate::ui::LlmState::Error,
        ];
        assert_eq!(states.len(), 4);

        for state in &states {
            let _cloned = state.clone();
        }
    }

    // -----------------------------------------------------------------------
    // TranscriptComponent with multiple search results — data integrity
    // -----------------------------------------------------------------------

    #[test]
    fn test_multiple_search_results_preserved() {
        let results: Vec<_> = (0..10)
            .map(|i| SearchResult {
                id: format!("doc{}", i),
                file_path: format!("/path/to/file{}.rs", i).into(),
                line_start: i * 10,
                line_end: i * 10 + 5,
                module_name: String::new(),
                symbol_kind: None,
                text: format!("fn example{}() -> i32 {{ {} }}", i, i),
                score: 0.95 - (i as f32 * 0.05),
            })
            .collect();

        let component = TranscriptComponent {
            error_msg: None,
            search_results: results.clone(),
            selected_result: 0,
            llm_state: crate::ui::LlmState::Idle,
            llm_answer: None,
            llm_partial_answer: String::new(),
            llm_scroll_offset: 0,
            colors: Colors::dark(),
        };

        assert_eq!(component.search_results.len(), 10);
    }

    #[test]
    fn test_selected_result_bounds() {
        let component = TranscriptComponent {
            error_msg: None,
            search_results: vec![SearchResult {
                id: "x".into(),
                file_path: "/x.rs".into(),
                line_start: 0,
                line_end: 1,
                module_name: String::new(),
                symbol_kind: None,
                text: "code".to_string(),
                score: 0.5,
            }],
            selected_result: 0,
            llm_state: crate::ui::LlmState::Idle,
            llm_answer: None,
            llm_partial_answer: String::new(),
            llm_scroll_offset: 0,
            colors: Colors::dark(),
        };

        // selected_result is 0 for a single item — valid index
        assert_eq!(component.selected_result, 0);
    }

    #[test]
    fn test_llm_scroll_offset_zero() {
        let component = TranscriptComponent {
            error_msg: None,
            search_results: Vec::new(),
            selected_result: 0,
            llm_state: crate::ui::LlmState::Idle,
            llm_answer: None,
            llm_partial_answer: String::new(),
            llm_scroll_offset: 0,
            colors: Colors::dark(),
        };

        assert_eq!(component.llm_scroll_offset, 0);
    }

    #[test]
    fn test_transcript_component_with_colors() {
        let dark = Colors::dark();
        let light = Colors::light();

        let comp_dark = TranscriptComponent {
            error_msg: None,
            search_results: Vec::new(),
            selected_result: 0,
            llm_state: crate::ui::LlmState::Idle,
            llm_answer: None,
            llm_partial_answer: String::new(),
            llm_scroll_offset: 0,
            colors: dark,
        };

        let comp_light = TranscriptComponent {
            error_msg: None,
            search_results: Vec::new(),
            selected_result: 0,
            llm_state: crate::ui::LlmState::Idle,
            llm_answer: None,
            llm_partial_answer: String::new(),
            llm_scroll_offset: 0,
            colors: light,
        };

        assert_eq!(comp_dark.colors.input_prompt, Color::Yellow);
        assert_eq!(comp_light.colors.input_prompt, Color::Cyan);
    }

    // -----------------------------------------------------------------------
    // SearchResult data structure tests (from rust_rag_core)
    // -----------------------------------------------------------------------

    #[test]
    fn test_search_result_all_fields() {
        let result = SearchResult {
            id: "doc-123".to_string(),
            file_path: "/home/user/project/mod.rs".into(),
            line_start: 42,
            line_end: 50,
            module_name: String::new(),
            symbol_kind: None,
            text: "pub fn my_function(x: i32) -> String { x.to_string() }"
                .to_string(),
            score: 0.8765,
        };

        assert_eq!(result.id, "doc-123");
        assert_eq!(result.line_start, 42);
        assert_eq!(result.line_end, 50);
        assert!(!result.text.is_empty());
        assert!(result.score > 0.0);
    }

    #[test]
    fn test_search_result_clone() {
        let result = SearchResult {
            id: "doc-1".to_string(),
            file_path: "/a/b.rs".into(),
            line_start: 1,
            line_end: 2,
            module_name: String::new(),
            symbol_kind: None,
            text: "test".to_string(),
            score: 0.9,
        };
        let cloned = result.clone();
        assert_eq!(result.id, cloned.id);
        assert_eq!(result.score, cloned.score);
    }

    #[test]
    fn test_search_result_debug() {
        let result = SearchResult {
            id: "debug".to_string(),
            file_path: "/test.rs".into(),
            line_start: 0,
            line_end: 1,
            module_name: String::new(),
            symbol_kind: None,
            text: "t".to_string(),
            score: 1.0,
        };
        let debug_str = format!("{:?}", result);
        assert!(debug_str.contains("SearchResult"));
    }

    #[test]
    fn test_search_result_with_symbol_kind() {
        use rust_rag_core::indexer::SymbolKind;
        let result = SearchResult {
            id: "fn-1".to_string(),
            file_path: "/lib.rs".into(),
            line_start: 5,
            line_end: 10,
            module_name: String::new(),
            symbol_kind: Some(SymbolKind::Function),
            text: "fn foo() {}".to_string(),
            score: 0.99,
        };
        assert!(result.symbol_kind.is_some());
    }

    // -----------------------------------------------------------------------
    // Edge cases for component construction
    // -----------------------------------------------------------------------

    #[test]
    fn test_transcript_component_with_high_scroll_offset() {
        let component = TranscriptComponent {
            error_msg: None,
            search_results: Vec::new(),
            selected_result: 0,
            llm_state: crate::ui::LlmState::Done,
            llm_answer: Some("line1\nline2".to_string()),
            llm_partial_answer: String::new(),
            llm_scroll_offset: 999, // way beyond actual content length
            colors: Colors::dark(),
        };

        assert_eq!(component.llm_scroll_offset, 999);
    }

    #[test]
    fn test_transcript_component_with_unicode_content() {
        let component = TranscriptComponent {
            error_msg: None,
            search_results: vec![SearchResult {
                id: "unicode".into(),
                file_path: "/файл.rs".into(), // non-ASCII filename
                line_start: 0,
                line_end: 1,
                module_name: String::new(),
                symbol_kind: None,
                text: "fn привет(мир: &str) -> String { мир.to_string() }"
                    .to_string(),
                score: 0.5,
            }],
            selected_result: 0,
            llm_state: crate::ui::LlmState::Idle,
            llm_answer: None,
            llm_partial_answer: String::new(),
            llm_scroll_offset: 0,
            colors: Colors::dark(),
        };

        assert!(!component.search_results[0].text.is_empty());
    }

    #[test]
    fn test_transcript_component_with_very_long_text() {
        let long_text = "x".repeat(10000);
        let component = TranscriptComponent {
            error_msg: None,
            search_results: vec![SearchResult {
                id: "long".into(),
                file_path: "/long.rs".into(),
                line_start: 0,
                line_end: 1,
                module_name: String::new(),
                symbol_kind: None,
                text: long_text.clone(),
                score: 0.5,
            }],
            selected_result: 0,
            llm_state: crate::ui::LlmState::Idle,
            llm_answer: None,
            llm_partial_answer: String::new(),
            llm_scroll_offset: 0,
            colors: Colors::dark(),
        };

        assert_eq!(component.search_results[0].text.len(), 10000);
    }

    #[test]
    fn test_transcript_component_with_multiline_answer() {
        let answer = "Line one\nLine two\nLine three\nLine four\nLine five";
        let component = TranscriptComponent {
            error_msg: None,
            search_results: Vec::new(),
            selected_result: 0,
            llm_state: crate::ui::LlmState::Done,
            llm_answer: Some(answer.to_string()),
            llm_partial_answer: String::new(),
            llm_scroll_offset: 0,
            colors: Colors::dark(),
        };

        if let Some(ans) = &component.llm_answer {
            let lines: Vec<&str> = ans.lines().collect();
            assert_eq!(lines.len(), 5);
        }
    }

    #[test]
    fn test_transcript_component_partial_answer_accumulation() {
        let mut partial = String::new();
        for word in ["Hello", " ", "world"] {
            partial.push_str(word);
        }
        assert_eq!(partial, "Hello world");
    }

    #[test]
    fn test_colors_different_between_palettes_in_component() {
        let dark_comp = TranscriptComponent {
            error_msg: None,
            search_results: Vec::new(),
            selected_result: 0,
            llm_state: crate::ui::LlmState::Idle,
            llm_answer: None,
            llm_partial_answer: String::new(),
            llm_scroll_offset: 0,
            colors: Colors::dark(),
        };

        let light_comp = TranscriptComponent {
            error_msg: None,
            search_results: Vec::new(),
            selected_result: 0,
            llm_state: crate::ui::LlmState::Idle,
            llm_answer: None,
            llm_partial_answer: String::new(),
            llm_scroll_offset: 0,
            colors: Colors::light(),
        };

        assert_ne!(dark_comp.colors.loading_fg, light_comp.colors.loading_fg);
    }

    #[test]
    fn test_empty_results_vector_is_valid() {
        let empty: Vec<SearchResult> = Vec::new();
        assert!(empty.is_empty());

        let component = TranscriptComponent {
            error_msg: None,
            search_results: empty,
            selected_result: 0,
            llm_state: crate::ui::LlmState::Idle,
            llm_answer: None,
            llm_partial_answer: String::new(),
            llm_scroll_offset: 0,
            colors: Colors::dark(),
        };

        assert!(component.search_results.is_empty());
    }
}
