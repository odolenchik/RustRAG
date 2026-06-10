use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use rust_rag_core::vector_store::SearchResult;

/// Renders the transcript area (search results + LLM answer/error).
pub struct TranscriptComponent {
    pub error_msg: Option<String>,
    pub search_results: Vec<SearchResult>,
    pub selected_result: usize,
    pub llm_state: super::LlmState,
    pub llm_answer: Option<String>,
    pub llm_partial_answer: String,
    pub llm_scroll_offset: usize,
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
                .style(Style::default().fg(Color::Red));
            frame.render_widget(block, area);

            let error_paragraph = Paragraph::new(Span::raw(format!("! {}", err)))
                .style(Style::default().fg(Color::Yellow));
            frame.render_widget(error_paragraph, area);
            return;
        }

        // Split: results on top, LLM answer on bottom
        let llm_height = match self.llm_state {
            super::LlmState::Loading => 2u16,
            super::LlmState::Done | super::LlmState::Error => 5,
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

            let list = List::new(items).highlight_style(Style::default().bg(Color::DarkGray));
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
                super::LlmState::Loading if !self.llm_partial_answer.is_empty() => {
                    let display_text = format!("\u{258A} {}", self.llm_partial_answer);
                    let llm_paragraph = Paragraph::new(Span::raw(display_text))
                        .style(Style::default().fg(Color::Green));
                    frame.render_widget(llm_paragraph, llm_rect);
                }
                super::LlmState::Loading => {
                    let loading_text = Paragraph::new(Span::raw("  LLM is thinking..."))
                        .style(Style::default().fg(Color::Yellow));
                    frame.render_widget(loading_text, llm_rect);
                }
                super::LlmState::Done => {
                    let ans_block = Block::default()
                        .title(" LLM Answer ")
                        .borders(Borders::ALL)
                        .style(Style::default().fg(Color::Green));
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
                            .style(Style::default().fg(Color::Green));
                        frame.render_widget(llm_paragraph, llm_rect);
                    }
                }
                super::LlmState::Error => {
                    let err_block = Block::default()
                        .title(" LLM Error ")
                        .borders(Borders::ALL)
                        .style(Style::default().fg(Color::Red));
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
                            .style(Style::default().fg(Color::Red));
                        frame.render_widget(llm_err, llm_rect);
                    }
                }
                _ => {}
            }
        }
    }
}
