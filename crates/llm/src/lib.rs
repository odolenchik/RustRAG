pub mod ollama_client;

use anyhow::Result;

/// Trait for LLM backends — allows swapping Ollama ↔ mistralrs later.
#[async_trait::async_trait]
pub trait ChatBackend: Send + Sync {
    /// Generate a chat completion given messages. Returns the response text.
    async fn complete(&self, system_prompt: &str, user_message: &str) -> Result<String>;

    /// Stream a chat completion (returns an async stream of chunks).
    fn complete_streaming<'a>(
        &'a self,
        system_prompt: &'a str,
        user_message: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send + 'a>>;
}
