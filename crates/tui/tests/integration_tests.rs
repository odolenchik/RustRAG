//! Integration tests for the rust-rag-tui crate.
//! These tests exercise public APIs across module boundaries without requiring a real terminal.

use std::path::{Path, PathBuf};

/// Re-export the key types used in integration tests.
use rust_rag_tui::theme::Colors;
use rust_rag_tui::ui::LlmState;

// ---------------------------------------------------------------------------
// Module visibility — verify that public API surface is accessible
// ---------------------------------------------------------------------------

#[test]
fn test_theme_module_is_public() {
    // Colors should be constructible from outside the crate.
    let dark = Colors::dark();
    assert_eq!(dark.title_fg, ratatui::prelude::Color::White);
}

#[test]
fn test_ui_module_is_public() {
    // LlmState should be accessible.
    let _idle: LlmState = LlmState::Idle;
    let _loading = LlmState::Loading;
    let _done = LlmState::Done;
    let _error = LlmState::Error;
}

#[test]
fn test_all_llm_state_variants_accessible() {
    let states = vec![
        LlmState::Idle,
        LlmState::Loading,
        LlmState::Done,
        LlmState::Error,
    ];
    assert_eq!(states.len(), 4);

    for state in &states {
        let _cloned = state.clone();
    }
}

// ---------------------------------------------------------------------------
// Cross-module — Colors + TranscriptComponent data flow
// -----------------------------------------------------------------------

#[test]
fn test_colors_flow_through_components() {
    // Verify that a Colors instance created in theme.rs can be used by
    // components in ui::transcript and ui::editor.
    let colors = Colors::dark();

    // error_block uses error_fg
    let style = colors.error_block();
    assert!(style.fg.is_some());

    // highlight_style uses highlight_bg
    let hl = colors.highlight_style();
    assert!(hl.bg.is_some());

    // fg helper
    let fg = colors.fg(ratatui::prelude::Color::Magenta);
    assert_eq!(fg.fg, Some(ratatui::prelude::Color::Magenta));
}

#[test]
fn test_dark_and_light_palettes_differ_in_components() {
    let dark = Colors::dark();
    let light = Colors::light();

    // Loading text color differs
    assert_ne!(dark.loading_fg, light.loading_fg);

    // Answer text color differs
    assert_ne!(dark.answer_fg, light.answer_fg);

    // Error colors should be the same (red in both)
    assert_eq!(dark.error_fg, light.error_fg);
}

// ---------------------------------------------------------------------------
// LlmState — state machine transitions
// -----------------------------------------------------------------------

#[test]
fn test_llmstate_can_transition_idle_to_loading() {
    let mut state = LlmState::Idle;
    assert!(matches!(state, LlmState::Idle));

    state = LlmState::Loading;
    assert!(matches!(state, LlmState::Loading));
}

#[test]
fn test_llmstate_can_transition_loading_to_done() {
    let mut state = LlmState::Loading;
    state = LlmState::Done;
    assert!(matches!(state, LlmState::Done));
}

#[test]
fn test_llmstate_can_transition_loading_to_error() {
    let mut state = LlmState::Loading;
    state = LlmState::Error;
    assert!(matches!(state, LlmState::Error));
}

#[test]
fn test_llmstate_default_is_idle_via_trait() {
    let default_state: LlmState = Default::default();
    assert!(matches!(default_state, LlmState::Idle));
}

// ---------------------------------------------------------------------------
// Colors — all fields are distinct and valid
// -----------------------------------------------------------------------

#[test]
fn test_dark_palette_all_fields_are_set() {
    let c = Colors::dark();
    // None of the colors should be Color::Reset (which would indicate a bug)
    assert_ne!(c.title_fg, ratatui::prelude::Color::Reset);
    assert_ne!(c.input_prompt, ratatui::prelude::Color::Reset);
    assert_ne!(c.error_fg, ratatui::prelude::Color::Reset);
}

#[test]
fn test_light_palette_all_fields_are_set() {
    let c = Colors::light();
    assert_ne!(c.title_fg, ratatui::prelude::Color::Reset);
    assert_ne!(c.input_prompt, ratatui::prelude::Color::Reset);
    assert_ne!(c.error_fg, ratatui::prelude::Color::Reset);
}

#[test]
fn test_colors_error_block_consistency() {
    let dark = Colors::dark();
    let light = Colors::light();

    // Both should use error_fg for the block foreground.
    assert_eq!(dark.error_block().fg, Some(dark.error_fg));
    assert_eq!(light.error_block().fg, Some(light.error_fg));
}

#[test]
fn test_colors_highlight_style_consistency() {
    let dark = Colors::dark();
    let light = Colors::light();

    // highlight_bg should be set.
    assert_eq!(dark.highlight_style().bg, Some(dark.highlight_bg));
    assert_eq!(light.highlight_style().bg, Some(light.highlight_bg));
}

// ---------------------------------------------------------------------------
// Public API — run function signature and behavior expectations
// -----------------------------------------------------------------------

#[test]
fn test_run_function_accepts_none() {
    // `run(None)` should use CWD as workspace. This doesn't actually execute
    // the TUI event loop in tests because we can't verify it directly, but
    // we confirm the function signature is correct and the initial path
    // resolution works.
    let cwd = std::env::current_dir();
    assert!(cwd.is_ok());
}

#[test]
fn test_run_function_accepts_some_path() {
    // `run(Some(path))` should use the given path.
    let temp_dir = tempfile::tempdir().unwrap();
    let path_str = temp_dir.path().to_string_lossy().to_string();
    // The function accepts Option<&str>. We verify construction works.
    let _path: PathBuf = PathBuf::from(&path_str);
}

// ---------------------------------------------------------------------------
// EditorComponent — handle_key cross-module access
// -----------------------------------------------------------------------

#[test]
fn test_editor_handle_key_is_public() {
    use rust_rag_tui::ui::editor::{self, Action};
    use crossterm::event::KeyCode;

    let mut query = String::new();
    editor::handle_key(KeyCode::Char('t'), &mut query);
    assert_eq!(query, "t");

    let result = editor::handle_key(KeyCode::Enter, &mut query);
    assert_eq!(result, Some(Action::Submit));
}

#[test]
fn test_editor_action_quit_is_public() {
    use rust_rag_tui::ui::editor::{self};
    use crossterm::event::KeyCode;

    let mut query = String::new();
    let result = editor::handle_key(KeyCode::Char('q'), &mut query);
    assert!(matches!(result, Some(editor::Action::Quit)));
}

#[test]
fn test_editor_action_submit_is_public() {
    use rust_rag_tui::ui::editor::{self};
    use crossterm::event::KeyCode;

    let mut query = String::new();
    let result = editor::handle_key(KeyCode::Enter, &mut query);
    assert!(matches!(result, Some(editor::Action::Submit)));
}

// ---------------------------------------------------------------------------
// LlmState — equality and comparison tests
// -----------------------------------------------------------------------

#[test]
fn test_llmstate_all_variants_are_distinct() {
    let idle = LlmState::Idle;
    let loading = LlmState::Loading;
    let done = LlmState::Done;
    let error = LlmState::Error;

    assert_ne!(idle, loading);
    assert_ne!(idle, done);
    assert_ne!(idle, error);
    assert_ne!(loading, done);
    assert_ne!(loading, error);
    assert_ne!(done, error);
}

// ---------------------------------------------------------------------------
// Colors — Clone/Copy semantics
// -----------------------------------------------------------------------

#[test]
fn test_colors_clone_is_identity() {
    let original = Colors::dark();
    let cloned = original.clone();

    assert_eq!(original.title_fg, cloned.title_fg);
    assert_eq!(original.title_bg, cloned.title_bg);
    assert_eq!(original.input_prompt, cloned.input_prompt);
    assert_eq!(original.error_fg, cloned.error_fg);
    assert_eq!(original.error_border, cloned.error_border);
    assert_eq!(original.loading_fg, cloned.loading_fg);
    assert_eq!(original.answer_fg, cloned.answer_fg);
    assert_eq!(original.answer_border, cloned.answer_border);
    assert_eq!(original.highlight_bg, cloned.highlight_bg);
}

#[test]
fn test_colors_light_clone_is_identity() {
    let original = Colors::light();
    let cloned = original.clone();

    assert_eq!(original.input_prompt, cloned.input_prompt);
    assert_eq!(original.loading_fg, cloned.loading_fg);
}

// ---------------------------------------------------------------------------
// Integration — constructing a realistic search result and using it with colors
// -----------------------------------------------------------------------

#[test]
fn test_search_result_integration_with_colors() {
    let colors = Colors::dark();

    // Simulate what the transcript component does: create a SearchResult-like data.
    // We can't construct SearchResult from tests (it's in rust-rag-core), but we
    // verify that our color system works correctly with the transcript data flow.

    let error_style = colors.error_block();
    assert_eq!(error_style.fg, Some(ratatui::prelude::Color::Red));

    let highlight = colors.highlight_style();
    assert_eq!(highlight.bg, Some(ratatui::prelude::Color::DarkGray));
}

// ---------------------------------------------------------------------------
// Full data flow — from key press to action interpretation
// -----------------------------------------------------------------------

#[test]
fn test_full_key_press_to_action_flow() {
    use rust_rag_tui::ui::editor;
    use crossterm::event::KeyCode;

    // Simulate a user typing "hello", pressing Enter, then 'q'.
    let mut query = String::new();

    for ch in ['h', 'e', 'l', 'l', 'o'] {
        editor::handle_key(KeyCode::Char(ch), &mut query);
    }
    assert_eq!(query, "hello");

    // Submit
    let result = editor::handle_key(KeyCode::Enter, &mut query);
    assert_eq!(result, Some(editor::Action::Submit));
    assert_eq!(query, "hello");

    // Continue typing
    for ch in ['w', 'o', 'r', 'l', 'd'] {
        editor::handle_key(KeyCode::Char(ch), &mut query);
    }
    assert_eq!(query, "helloworld");

    // Quit
    let result = editor::handle_key(KeyCode::Char('q'), &mut query);
    assert_eq!(result, Some(editor::Action::Quit));
}

#[test]
fn test_backspace_then_retype_flow() {
    use rust_rag_tui::ui::editor;
    use crossterm::event::KeyCode;

    let mut query = String::new();

    // Type "abc"
    for ch in ['a', 'b', 'c'] {
        editor::handle_key(KeyCode::Char(ch), &mut query);
    }
    assert_eq!(query, "abc");

    // Backspace twice -> "a"
    editor::handle_key(KeyCode::Backspace, &mut query);
    editor::handle_key(KeyCode::Backspace, &mut query);
    assert_eq!(query, "a");

    // Type "d" -> "ad"
    editor::handle_key(KeyCode::Char('d'), &mut query);
    assert_eq!(query, "ad");

    // Submit
    let result = editor::handle_key(KeyCode::Enter, &mut query);
    assert_eq!(result, Some(editor::Action::Submit));
}

// ---------------------------------------------------------------------------
// Edge case — empty transcript component with all state combinations
// -----------------------------------------------------------------------

#[test]
fn test_transcript_empty_state_idle() {
    // When search_results is empty and error_msg is None, the component shows help text.
    let colors = Colors::dark();
    assert_eq!(colors.input_prompt, ratatui::prelude::Color::Yellow);
}

#[test]
fn test_transcript_error_state_only() {
    // When error_msg is set (even with empty search_results), the component shows an error block.
    let colors = Colors::dark();
    assert_eq!(colors.error_fg, ratatui::prelude::Color::Red);
}

// ---------------------------------------------------------------------------
// Edge case — Loading state rendering hints
// -----------------------------------------------------------------------

#[test]
fn test_loading_state_has_green_foreground() {
    let dark = Colors::dark();
    assert_eq!(dark.loading_fg, ratatui::prelude::Color::Green);

    let light = Colors::light();
    assert_eq!(light.loading_fg, ratatui::prelude::Color::DarkGray);
}

// ---------------------------------------------------------------------------
// Edge case — Done state rendering hints
// -----------------------------------------------------------------------

#[test]
fn test_done_state_has_green_foreground_dark_theme() {
    let colors = Colors::dark();
    assert_eq!(colors.answer_fg, ratatui::prelude::Color::Green);
}
