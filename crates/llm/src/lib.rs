pub mod ollama_client;
pub mod validation;

use anyhow::Result;
use futures_core::Stream;

/// Trait for LLM backends — allows swapping Ollama ↔ mistralrs later.
#[async_trait::async_trait]
pub trait ChatBackend: Send + Sync {
    /// Generate a chat completion given messages. Returns the response text.
    async fn complete(&self, system_prompt: &str, user_message: &str) -> Result<String>;

    /// Stream a chat completion (returns an async stream of chunks).
    /// Accumulates all streamed chunks into a single string result.
    fn complete_streaming<'a>(
        &'a self,
        system_prompt: &'a str,
        user_message: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send + 'a>> {
        let stream = self.complete_stream_chunks(system_prompt, user_message);
        Box::pin(async move {
            use futures_util::StreamExt;
            let mut output = String::new();
            let mut s = stream;
            while let Some(chunk) = s.next().await {
                match chunk {
                    Ok(text) => output.push_str(&text),
                    Err(e) => return Err(e),
                }
            }
            Ok(output)
        })
    }

    /// Stream a chat completion as a futures_core::Stream of text fragments.
    fn complete_stream_chunks<'a>(
        &'a self,
        system_prompt: &'a str,
        user_message: &'a str,
    ) -> std::pin::Pin<Box<dyn Stream<Item = Result<String>> + Send + 'a>>;
}
