pub mod editor;
pub mod transcript;

/// LLM state tracking (shared between components).
#[derive(Clone, Debug, PartialEq)]
pub enum LlmState {
    Idle,
    Loading,
    Done,  // Answer stored in llm_answer field
    Error, // Error stored in llm_answer field
}

impl Default for LlmState {
    fn default() -> Self {
        Self::Idle
    }
}
