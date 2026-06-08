use super::{ChatBackend, Result};

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
        } else if base_url.ends_with("/chat/completions") || base_url.ends_with("/v1/chat/completions") {
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
        let model = model.unwrap_or_else(|| "Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-IQ3_M.gguf".to_string());

        LlmClient::new(&endpoint, &model)
    }

    /// Convenience synchronous wrapper — reads config from disk each time.
    pub fn chat(system_prompt: &str, user_message: &str) -> Result<String> {
        let client = LlmClient::default();
        tokio::runtime::Runtime::new()?.block_on(client.complete(system_prompt, user_message))
    }

    /// Convenience wrapper that uses explicit endpoint/model (bypasses config).
    pub fn chat_with(endpoint: &str, model: &str, system_prompt: &str, user_message: &str) -> Result<String> {
        let client = LlmClient::new(endpoint, model);
        tokio::runtime::Runtime::new()?.block_on(client.complete(system_prompt, user_message))
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

        let response = self.http_client.post(url)
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
        Box::pin(async move {
            self.complete(system_prompt, user_message).await
        })
    }
}

/// Backward-compatible alias — points to the new client.
pub type OllamaClient = LlmClient;
