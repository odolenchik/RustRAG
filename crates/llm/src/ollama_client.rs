use super::{ChatBackend, Result};
use futures_core::Stream;
use futures_util::stream::StreamExt;

/// SSE data chunk parsed from streaming response.
#[derive(Debug)]
struct SseChunk {
    content: String,
}

impl SseChunk {
    fn parse(line: &str) -> Option<Self> {
        // Standard SSE format: "data: {...}" or just "{...}" for llama.cpp
        let json_str = line.strip_prefix("data: ").or(Some(line))?;
        if json_str.trim() == "[DONE]" {
            return None;
        }
        let parsed: serde_json::Value = match serde_json::from_str(json_str) {
            Ok(v) => v,
            Err(_) => return None,
        };
        // Extract delta.content from choices[0].delta.content (OpenAI/Ollama format)
        if let Some(content) = parsed["choices"][0]["delta"]["content"].as_str() {
            Some(SseChunk {
                content: content.to_string(),
            })
        } else if let Some(text_val) = parsed["choices"][0]["text"].as_str() {
            // llama.cpp may return text field directly
            Some(SseChunk {
                content: text_val.to_string(),
            })
        } else {
            None
        }
    }
}

fn stream_chunks<'a>(
    client: &'a LlmClient,
    system_prompt: &'a str,
    user_message: &'a str,
) -> impl Stream<Item = Result<String>> + Send + 'a {
    async_stream::stream! {
        let url = client.base_url.as_str();

        // Build streaming request body — use raw JSON since we need to set stream=true
        let request_body = serde_json::json!({
            "model": client.model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_message},
            ],
            "stream": true,
        });

        let response = match client.http_client.post(url)
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await {
                Ok(resp) => resp,
                Err(e) => { yield Err(e.into()); return; }
        };

        if !response.status().is_success() {
            let status = response.status();
            let error_text = match response.text().await {
                Ok(t) => t,
                Err(_) => "unknown".to_string(),
            };
            yield Err(anyhow::anyhow!("LLM API error ({}): {}", status, error_text));
            return;
        }

        // Iterate over the byte stream and parse SSE chunks
        let mut buf = String::new();
        let mut stream_boxed = Box::pin(response.bytes_stream());

        while let Some(chunk_result) = stream_boxed.next().await {
            let chunk_bytes = match chunk_result {
                Ok(b) => b,
                Err(e) => {
                    yield Err(anyhow::anyhow!("Stream read error: {}", e));
                    break;
                }
            };
            let text = match String::from_utf8(chunk_bytes.to_vec()) {
                Ok(t) => t,
                Err(_) => continue,
            };
            buf.push_str(&text);

            // Process complete lines from buffer
            while let Some(newline_pos) = buf.find('\n') {
                let line = buf[..newline_pos].trim().to_string();
                // newline_pos is byte offset of '\n', consume up to and including it
                buf.drain(..=newline_pos);

                if let Some(parsed) = SseChunk::parse(&line) {
                    yield Ok(parsed.content);
                }
            }
        }
    }
}

/// LLM client that works with both Ollama and llama.cpp (OpenAI-compatible) backends.
pub struct LlmClient {
    base_url: url::Url,
    model: String,
    http_client: reqwest::Client,
}

impl LlmClient {
    /// Create a new client with the given base URL and model.
    pub fn new(base_url: &str, model: &str) -> Self {
        let url = if !base_url.starts_with("http") {
            format!("http://{}/chat/completions", base_url)
        } else if base_url.ends_with("/chat/completions")
            || base_url.ends_with("/v1/chat/completions")
        {
            // Strip trailing path, keep only origin
            let url = base_url.trim_end_matches('/');
            let slash_pos = url.rfind('/').map(|i| i + 1).unwrap_or(url.len());
            format!("{}/chat/completions", &url[..slash_pos])
        } else {
            format!("{}/chat/completions", base_url)
        };

        LlmClient {
            base_url: url.parse().expect("Invalid base URL"),
            model: model.to_string(),
            http_client: reqwest::Client::new(),
        }
    }

    /// Default: uses config file (endpoint/model from .rustrag.toml), falls back to env var, then hardcoded defaults.
    pub fn default() -> Self {
        let cfg = rust_rag_core::config::Config::find().ok();

        // Resolve endpoint with priority: config > LLAMA_ENDPOINT env > hard-coded default
        let endpoint = (|| -> Option<String> {
            if let Some(ref c) = cfg {
                if let Some(ep) = &c.llm_config().endpoint {
                    return Some(ep.clone());
                }
            }
            std::env::var("LLAMA_ENDPOINT").ok()
        })();

        // Resolve model with priority: config > LLAMA_MODEL env > hard-coded default
        let model = (|| -> Option<String> {
            if let Some(ref c) = cfg {
                if let Some(m) = &c.llm_config().model {
                    return Some(m.clone());
                }
            }
            std::env::var("LLAMA_MODEL").ok()
        })();

        let endpoint = endpoint.unwrap_or_else(|| "http://localhost:8080".to_string());
        let model = model.unwrap_or_else(|| {
            "Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-IQ3_M.gguf".to_string()
        });

        LlmClient::new(&endpoint, &model)
    }

    fn sync_runtime() -> &'static tokio::runtime::Handle {
        // SAFETY: LazyLock guarantees initialization happens exactly once before first access.
        // All callers use `.block_on()` which requires a Handle, not the Runtime itself.
        use std::sync::LazyLock;
        static RT: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create shared Tokio runtime for LLM calls")
        });
        RT.handle()
    }

    /// Convenience synchronous wrapper — reads config from disk each time.
    pub fn chat(system_prompt: &str, user_message: &str) -> Result<String> {
        let client = LlmClient::default();
        Self::sync_runtime().block_on(client.complete(system_prompt, user_message))
    }

    /// Convenience wrapper that uses explicit endpoint/model (bypasses config).
    pub fn chat_with(
        endpoint: &str,
        model: &str,
        system_prompt: &str,
        user_message: &str,
    ) -> Result<String> {
        let client = LlmClient::new(endpoint, model);
        Self::sync_runtime().block_on(client.complete(system_prompt, user_message))
    }
}

#[async_trait::async_trait]
impl ChatBackend for LlmClient {
    async fn complete(&self, system_prompt: &str, user_message: &str) -> Result<String> {
        // base_url already contains the full endpoint path (e.g. http://localhost:8080/chat/completions)
        let url = self.base_url.as_str();

        // llama.cpp / OpenAI-compatible API format
        let request = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_message},
            ],
            "stream": false,
        });

        let response = self
            .http_client
            .post(url)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await?;
            anyhow::bail!("LLM API error ({}): {}", status, error_text);
        }

        let json: serde_json::Value = response.json().await?;
        let content = json["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("No message.content in LLM response"))?;

        Ok(content.to_string())
    }

    fn complete_streaming<'a>(
        &'a self,
        system_prompt: &'a str,
        user_message: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(async move { self.complete(system_prompt, user_message).await })
    }

    fn complete_stream_chunks<'a>(
        &'a self,
        system_prompt: &'a str,
        user_message: &'a str,
    ) -> std::pin::Pin<Box<dyn Stream<Item = Result<String>> + Send + 'a>> {
        Box::pin(stream_chunks(self, system_prompt, user_message))
    }
}

/// Backward-compatible alias — points to the new client.
pub type OllamaClient = LlmClient;
