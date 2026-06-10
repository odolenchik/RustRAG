use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::prelude::*;
// Rectangle = Rect (from prelude)
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use rust_rag_core::vector_store::{SearchResult, VectorStore};
use rust_rag_llm::ChatBackend;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::LazyLock;
use std::time::Duration;

/// Shared static runtime for TUI LLM calls — created once and reused.
static TUI_RT: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create shared Tokio runtime for TUI")
});

/// Load top_k from config (defaults to 5).
fn load_top_k(workspace_root: &std::path::Path) -> usize {
    rust_rag_core::config::Config::load(workspace_root)
        .ok()
        .and_then(|c| Some(c.llm_config().top_k))
        .unwrap_or(5)
}

// ---------------------------------------------------------------------------
// App state machine
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq)]
pub enum LlmState {
    Idle,
    Loading,
    Done,  // Answer stored in `llm_answer` field
    Error, // Error stored in `llm_answer` field
}

impl Default for LlmState {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Clone, PartialEq)]
pub enum AppState {
    Idle,
    Searching,
    Results,
}

#[derive(Debug)]
enum TuiEvent {
    LlmChunk(String),
    LlmDone,
    LlmError(String),
}

pub struct App {
    running: bool,
    query: String,
    search_results: Vec<SearchResult>,
    selected_result: usize,
    llm_state: LlmState,
    llm_answer: Option<String>,
    llm_partial_answer: String, // accumulated text during streaming
    error_msg: Option<String>,
    workspace_path: PathBuf,
    app_state: AppState,
    tx: mpsc::Sender<TuiEvent>,
    rx: std::sync::Arc<std::sync::Mutex<mpsc::Receiver<TuiEvent>>>,
    // Scroll offset for LLM answer area.
    llm_scroll_offset: usize,
}

impl App {
    pub fn new(workspace_root: &std::path::Path) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            running: true,
            query: String::new(),
            search_results: Vec::new(),
            selected_result: 0,
            llm_state: LlmState::Idle,
            llm_answer: None,
            llm_partial_answer: String::new(),
            error_msg: None,
            workspace_path: workspace_root.to_path_buf(),
            app_state: AppState::Idle,
            llm_scroll_offset: 0,
            tx,
            rx: std::sync::Arc::new(std::sync::Mutex::new(rx)),
        }
    }

    fn run_search(&mut self) {
        let query = std::mem::take(&mut self.query);
        if query.trim().is_empty() {
            return;
        }
        let tx2 = self.tx.clone();

        let workspace_root = self.workspace_path.clone();
        let store_path = workspace_root.join(".rustrag");

        let index_path = store_path.join("index.jsonl");
        if !index_path.exists() {
            self.error_msg = Some(format!(
                "No index found at {}. Run `rust-rag index <path>` first.",
                workspace_root.display()
            ));
            return;
        }

        let embedding = match rust_rag_core::embedding::embed(&query) {
            Ok(v) => v,
            Err(e) => {
                self.error_msg = Some(format!("Embedding error: {}", e));
                return;
            }
        };

        let store = match VectorStore::open(&store_path) {
            Ok(s) => s,
            Err(e) => {
                self.error_msg = Some(format!("Vector store error: {}", e));
                return;
            }
        };

        let top_k = load_top_k(&self.workspace_path);
        let results = match store.hybrid_search(&embedding, &query, top_k, 0.7, None) {
            Ok(r) => r,
            Err(e) => {
                self.error_msg = Some(format!("Search error: {}", e));
                return;
            }
        };

        self.search_results = results;
        self.selected_result = 0;
        self.app_state = AppState::Results;
        self.llm_state = LlmState::Loading;

        let system_prompt = "You are a Rust code analysis assistant. Answer questions based on the provided code snippets.";
        let context: String = self
            .search_results
            .iter()
            .map(|r| r.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        let full_message = format!("Question: {}\n\nRelevant code:\n{}", query, context);

        std::thread::spawn(move || {
            // LLM client reads endpoint/model from .rustrag.toml (via Config::find())
            // Use the shared runtime instead of creating a new one per request.
            let client = rust_rag_llm::ollama_client::LlmClient::default();

            TUI_RT.block_on(async {
                let mut stream = client.complete_stream_chunks(&system_prompt, &full_message);
                loop {
                    let chunk_result = futures_util::stream::StreamExt::next(&mut stream).await;
                    match chunk_result {
                        Some(Ok(text)) => {
                            // Send partial text to TUI for live display
                            if tx2.send(TuiEvent::LlmChunk(text)).is_err() {
                                break;
                            }
                        }
                        Some(Err(e)) => {
                            let _ = tx2.send(TuiEvent::LlmError(format!("{}", e)));
                            break;
                        }
                        None => break, // stream exhausted — done normally
                    }
                }
            });

            // After stream completes, send final Done event with accumulated answer
            let _ = tx2.send(TuiEvent::LlmDone);
        });
    }

    fn handle_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Char('q') | KeyCode::Char('Q') => self.running = false,
            KeyCode::Char(c) => self.query.push(c),
            KeyCode::Backspace => {
                self.query.pop();
            }
            KeyCode::Enter => {
                if !self.query.is_empty() {
                    self.run_search();
                }
            }
            KeyCode::Esc => {
                self.search_results.clear();
                self.selected_result = 0;
                self.llm_state = LlmState::Idle;
                self.app_state = AppState::Idle;
            }
            // PageUp / PageDown: scroll results list and LLM answer area.
            KeyCode::PageUp | KeyCode::BackTab => {
                let page_size = 5;
                if self.selected_result > page_size {
                    self.selected_result -= page_size;
                } else {
                    self.selected_result = 0;
                }
                // Also scroll LLM answer area.
                if !self.search_results.is_empty()
                    && (self.llm_state == LlmState::Done || self.llm_state == LlmState::Error)
                {
                    let step = 3;
                    if self.llm_scroll_offset > step {
                        self.llm_scroll_offset -= step;
                    } else {
                        self.llm_scroll_offset = 0;
                    }
                }
            }
            KeyCode::PageDown => {
                // Scroll results list.
                let page_size = 5;
                let max_idx = self.search_results.len().saturating_sub(1);
                if self.selected_result + page_size < max_idx {
                    self.selected_result += page_size;
                } else {
                    self.selected_result = max_idx;
                }
                // Also scroll LLM answer area.
                if !self.search_results.is_empty()
                    && (self.llm_state == LlmState::Done || self.llm_state == LlmState::Error)
                {
                    let _max_scroll = self.llm_answer.as_ref().map_or(0, |a| a.lines().count());
                    let step = 3;
                    if self.llm_scroll_offset + step < _max_scroll.saturating_sub(1) {
                        self.llm_scroll_offset += step;
                    } else {
                        self.llm_scroll_offset = _max_scroll.saturating_sub(1);
                    }
                }
            }
            // Home / End for quick navigation
            KeyCode::Home => {
                self.selected_result = 0;
            }
            KeyCode::End => {
                let max_idx = self.search_results.len().saturating_sub(1);
                self.selected_result = max_idx;
            }
            _ => {}
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        let _area = frame.area();

        // Overall layout: title bar | output (results + LLM) | input line
        let main_chunks = Layout::vertical([
            Constraint::Length(1), // Title bar
            Constraint::Min(1),    // Output — split into results + LLM below
            Constraint::Length(1), // Input line (prompt)
        ])
        .split(frame.area());

        // --- Title bar ---
        let title = Span::styled(
            " RustRAG - Interactive Chat ",
            Style::default().fg(Color::White).bg(Color::Blue),
        );
        frame.render_widget(title, main_chunks[0]);

        // --- Process events from background thread ---
        if let Ok(rx) = self.rx.lock() {
            while let Ok(event) = rx.try_recv() {
                match event {
                    TuiEvent::LlmChunk(chunk) => {
                        // Streaming chunk — append partial text and trigger redraw
                        self.llm_partial_answer.push_str(&chunk);
                    }
                    TuiEvent::LlmDone => {
                        // Stream complete — move accumulated answer into llm_answer
                        if !self.llm_partial_answer.is_empty() {
                            self.llm_answer = Some(self.llm_partial_answer.clone());
                            self.llm_state = LlmState::Done;
                        }
                        self.llm_partial_answer.clear();
                    }
                    TuiEvent::LlmError(err) => {
                        self.llm_state = LlmState::Error;
                        self.llm_answer = Some(format!("LLM Error: {}", err));
                    }
                }
            }
        }

        // --- Output area ---
        if self.error_msg.is_some() {
            let error_text = format!("! {}", self.error_msg.as_ref().unwrap());
            let block = Block::default()
                .title(" Error ")
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::Red));
            frame.render_widget(block, main_chunks[1]);

            let error_paragraph =
                Paragraph::new(Span::raw(&error_text)).style(Style::default().fg(Color::Yellow));
            frame.render_widget(error_paragraph, main_chunks[1]);
        } else if self.search_results.is_empty() {
            let help_text = "Type a question and press Enter. Press 'q' to quit.";
            let block = Block::default().borders(Borders::ALL);
            frame.render_widget(block, main_chunks[1]);

            let paragraph = Paragraph::new(Span::raw(help_text));
            frame.render_widget(paragraph, main_chunks[1]);
        } else {
            // Split output area: top for results list, bottom for LLM answer
            let output_area = main_chunks[1];
            let llm_height = match &self.llm_state {
                LlmState::Loading => 2u16,
                LlmState::Done | LlmState::Error => 5,
                _ => 0,
            };
            let results_h = output_area.height.saturating_sub(1 + llm_height);

            // Top: search results (scrollable)
            if results_h > 2 {
                let results_rect = Rect {
                    x: output_area.x,
                    y: output_area.y,
                    width: output_area.width,
                    height: results_h,
                };

                let max_items = (results_h.saturating_sub(4) as usize).min(5); // at most 5 results shown at once
                let items: Vec<ListItem> = self
                    .search_results
                    .iter()
                    .skip(self.selected_result.saturating_sub(max_items))
                    .take(max_items)
                    .enumerate()
                    .map(|(_, r)| {
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

            // Bottom: LLM status or answer (separate area below results)
            if llm_height > 0 {
                let llm_y = output_area.y + results_h + 1;
                let remaining = output_area.height.saturating_sub(llm_y - output_area.y);
                if remaining == 0 || remaining < 2 {
                    return;
                }

                let llm_rect = Rect {
                    x: output_area.x,
                    y: llm_y,
                    width: output_area.width.min(80),
                    height: llm_height.min(remaining),
                };

                if self.llm_state == LlmState::Loading && !self.llm_partial_answer.is_empty() {
                    // Show streaming partial answer with a blinking cursor indicator
                    let display_text = format!("\u{258A} {}", self.llm_partial_answer);
                    let llm_paragraph = Paragraph::new(Span::raw(display_text))
                        .style(Style::default().fg(Color::Green));
                    frame.render_widget(llm_paragraph, llm_rect);
                } else if self.llm_state == LlmState::Loading {
                    let loading_text = Paragraph::new(Span::raw("  LLM is thinking..."))
                        .style(Style::default().fg(Color::Yellow));
                    frame.render_widget(loading_text, llm_rect);
                } else if self.llm_state == LlmState::Done {
                    // Title bar for answer section
                    let ans_block = Block::default()
                        .title(" LLM Answer ")
                        .borders(Borders::ALL)
                        .style(Style::default().fg(Color::Green));
                    frame.render_widget(ans_block, llm_rect);

                    if let Some(answer) = &self.llm_answer {
                        // Show answer with scroll offset — only display lines that fit.
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
                        // Show scroll indicator.
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
                } else if self.llm_state == LlmState::Error {
                    let err_block = Block::default()
                        .title(" LLM Error ")
                        .borders(Borders::ALL)
                        .style(Style::default().fg(Color::Red));
                    frame.render_widget(err_block, llm_rect);

                    if let Some(ref err_msg) = self.llm_answer {
                        // Also scrollable for long error messages.
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
            }
        }

        // --- Input line --- prompt + query text
        let prompt_text = format!("> {}", self.query);
        let input_paragraph =
            Paragraph::new(Span::raw(prompt_text)).style(Style::default().fg(Color::Yellow));
        frame.render_widget(input_paragraph, main_chunks[2]);

        // Position cursor at end of typed query
        let cursor_x = 1 + (self.query.len() as u16);
        let max_x = main_chunks[2].x + main_chunks[2].width.saturating_sub(1);
        frame.set_cursor_position(Position {
            x: cursor_x.min(max_x),
            y: main_chunks[2].y,
        });
    }

    pub fn run(&mut self) -> Result<()> {
        let mut terminal = ratatui::init();

        while self.running {
            terminal.draw(|frame| self.draw(frame))?;

            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key_event) = event::read()? {
                    // Only process key presses (not autorelease/release)
                    if key_event.kind == KeyEventKind::Press {
                        match key_event.code {
                            KeyCode::Up => {
                                if self.selected_result > 0 {
                                    self.selected_result -= 1;
                                }
                            }
                            KeyCode::Down => {
                                let max_idx = self.search_results.len().saturating_sub(1);
                                if self.selected_result < max_idx {
                                    self.selected_result += 1;
                                }
                            }
                            key => self.handle_key(key),
                        }
                    }
                }
            }
        }

        ratatui::restore();
        Ok(())
    }
}

/// Entry point - creates App and runs the event loop.
pub fn run_app(workspace_root: &std::path::Path) -> Result<()> {
    let mut app = App::new(workspace_root);
    app.run()
}
