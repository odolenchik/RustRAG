use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::prelude::*;
use rust_rag_llm::ChatBackend;
use std::sync::LazyLock;
use std::time::Duration;

// Re-export for backward compatibility
pub use super::ui::LlmState;

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
        .map(|c| c.llm_config().top_k)
        .unwrap_or(5)
}

// ---------------------------------------------------------------------------
// App state machine
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq)]
pub enum AppState {
    Idle,
    Searching,
    Results,
}

#[allow(clippy::enum_variant_names)]
#[derive(Debug)]
enum TuiEvent {
    LlmChunk(String),
    LlmDone,
    LlmError(String),
}

pub struct App {
    running: bool,
    query: String,
    search_results: Vec<rust_rag_core::vector_store::SearchResult>,
    selected_result: usize,
    llm_state: super::ui::LlmState,
    llm_answer: Option<String>,
    llm_partial_answer: String, // accumulated text during streaming
    error_msg: Option<String>,
    workspace_path: std::path::PathBuf,
    app_state: AppState,
    tx: std::sync::mpsc::Sender<TuiEvent>,
    rx: std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<TuiEvent>>>,
    // Scroll offset for LLM answer area.
    llm_scroll_offset: usize,
}

impl App {
    pub fn new(workspace_root: &std::path::Path) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        Self {
            running: true,
            query: String::new(),
            search_results: Vec::new(),
            selected_result: 0,
            llm_state: super::ui::LlmState::Idle,
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

        let store = match rust_rag_core::vector_store::VectorStore::open(&store_path) {
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
        self.llm_state = super::ui::LlmState::Loading;

        let system_prompt = rust_rag_core::constants::DEFAULT_SYSTEM_PROMPT;
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
            let client = rust_rag_llm::ollama_client::LlmClient::from_config();

            TUI_RT.block_on(async {
                let mut stream = client.complete_stream_chunks(system_prompt, &full_message);
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
        // Delegate to editor component for input handling
        match super::ui::editor::handle_key(key, &mut self.query) {
            Some(super::ui::editor::Action::Quit) => self.running = false,
            Some(super::ui::editor::Action::Submit) | None => {}
        }
        if matches!(key, KeyCode::Enter) && !self.query.is_empty() {
            self.run_search();
        } else {
            // Already handled by editor; process navigation separately below
        }

        // Navigation keys not handled by editor component
        match key {
            KeyCode::Esc => {
                self.search_results.clear();
                self.selected_result = 0;
                self.llm_state = super::ui::LlmState::Idle;
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
                    && (self.llm_state == super::ui::LlmState::Done
                        || self.llm_state == super::ui::LlmState::Error)
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
                    && (self.llm_state == super::ui::LlmState::Done
                        || self.llm_state == super::ui::LlmState::Error)
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
        // Process events from background thread first
        if let Ok(rx) = self.rx.lock() {
            while let Ok(event) = rx.try_recv() {
                match event {
                    TuiEvent::LlmChunk(chunk) => {
                        self.llm_partial_answer.push_str(&chunk);
                    }
                    TuiEvent::LlmDone => {
                        if !self.llm_partial_answer.is_empty() {
                            self.llm_answer = Some(self.llm_partial_answer.clone());
                            self.llm_state = super::ui::LlmState::Done;
                        }
                        self.llm_partial_answer.clear();
                    }
                    TuiEvent::LlmError(err) => {
                        self.llm_state = super::ui::LlmState::Error;
                        self.llm_answer = Some(format!("LLM Error: {}", err));
                    }
                }
            }
        }

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

        // --- Output area: delegate to transcript component ---
        let transcript_data = super::ui::transcript::TranscriptComponent {
            error_msg: self.error_msg.clone(),
            search_results: std::mem::take(&mut self.search_results),
            selected_result: self.selected_result,
            llm_state: self.llm_state.clone(),
            llm_answer: self.llm_answer.clone(),
            llm_partial_answer: std::mem::take(&mut self.llm_partial_answer),
            llm_scroll_offset: self.llm_scroll_offset,
        };
        transcript_data.draw(frame, main_chunks[1]);

        // Restore search_results (TranscriptComponent took ownership via take)
        if !transcript_data.search_results.is_empty() {
            self.search_results = transcript_data.search_results;
        }

        // --- Input line: delegate to editor component ---
        let editor_data = super::ui::editor::EditorComponent {
            query: std::mem::take(&mut self.query),
        };
        editor_data.draw(frame, main_chunks[2]);
        self.query = editor_data.query;
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
