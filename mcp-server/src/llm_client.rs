// BYO LLM client for OpenAI-compatible providers.
//
// Reads LLM_BASE_URL, LLM_API_KEY, LLM_MODEL from environment.
// Optional: LLM_TIMEOUT_MS, LLM_MAX_TOKENS, LLM_TEMPERATURE.

use anyhow::{anyhow, Result};
use serde_json::Value;
use std::time::Duration;

/// Known LLM error codes returned by this module.
pub const LLM_NOT_CONFIGURED: &str = "LLM_NOT_CONFIGURED";
pub const LLM_AUTH_FAILED: &str = "LLM_AUTH_FAILED";
pub const LLM_RATE_LIMITED: &str = "LLM_RATE_LIMITED";
pub const LLM_TIMEOUT: &str = "LLM_TIMEOUT";
pub const LLM_INVALID_JSON: &str = "LLM_INVALID_JSON";

/// Minimal OpenAI-compatible LLM client.
///
/// Configured entirely from environment variables. Supports any provider
/// that exposes a `/chat/completions` endpoint with the OpenAI request shape.
///
/// Fields are private to prevent accidental exposure of `api_key`.
/// Use [`LlmClient::from_env`] to construct.
pub struct LlmClient {
    base_url: String,
    api_key: String,
    model: String,
    timeout: Duration,
    max_tokens: Option<u32>,
    temperature: Option<f64>,
    http_client: reqwest::Client,
}

impl std::fmt::Debug for LlmClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmClient")
            .field("base_url", &self.base_url)
            .field("api_key", &redact_key(&self.api_key))
            .field("model", &self.model)
            .field("timeout", &self.timeout)
            .field("max_tokens", &self.max_tokens)
            .field("temperature", &self.temperature)
            .finish()
    }
}

/// Redact an API key for safe display, keeping only the first 4 characters.
///
/// Uses char-based iteration to avoid panicking on multi-byte UTF-8 boundaries.
fn redact_key(key: &str) -> String {
    let prefix: String = key.chars().take(4).collect();
    if prefix.len() < key.chars().count() {
        format!("{}***", prefix)
    } else {
        "***".to_string()
    }
}

impl LlmClient {
    /// Create an LlmClient from environment variables.
    ///
    /// Required: LLM_BASE_URL, LLM_API_KEY, LLM_MODEL.
    /// Optional: LLM_TIMEOUT_MS (default 60000), LLM_MAX_TOKENS, LLM_TEMPERATURE.
    ///
    /// Returns an error containing `LLM_NOT_CONFIGURED` if any required var is missing.
    pub fn from_env() -> Result<Self> {
        let base_url = std::env::var("LLM_BASE_URL").map_err(|_| {
            anyhow!(
                "{}: LLM_BASE_URL environment variable is required",
                LLM_NOT_CONFIGURED
            )
        })?;
        let api_key = std::env::var("LLM_API_KEY").map_err(|_| {
            anyhow!(
                "{}: LLM_API_KEY environment variable is required",
                LLM_NOT_CONFIGURED
            )
        })?;
        let model = std::env::var("LLM_MODEL").map_err(|_| {
            anyhow!(
                "{}: LLM_MODEL environment variable is required",
                LLM_NOT_CONFIGURED
            )
        })?;

        let timeout_ms: u64 = std::env::var("LLM_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60_000);

        let max_tokens: Option<u32> = std::env::var("LLM_MAX_TOKENS")
            .ok()
            .and_then(|v| v.parse().ok());

        let temperature: Option<f64> = std::env::var("LLM_TEMPERATURE")
            .ok()
            .and_then(|v| v.parse().ok());

        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .map_err(|e| anyhow!("Failed to build HTTP client for LLM: {}", e))?;

        Ok(Self {
            base_url,
            api_key,
            model,
            timeout: Duration::from_millis(timeout_ms),
            max_tokens,
            temperature,
            http_client,
        })
    }

    /// Build an OpenAI-compatible chat/completions JSON payload.
    ///
    /// The `system_prompt` is sent as the system message and `user_content`
    /// as the user message.
    pub fn build_chat_payload(&self, system_prompt: &str, user_content: &str) -> Value {
        let mut payload = serde_json::json!({
            "model": self.model,
            "messages": [
                {
                    "role": "system",
                    "content": system_prompt
                },
                {
                    "role": "user",
                    "content": user_content
                }
            ]
        });

        if let Some(max_tokens) = self.max_tokens {
            payload["max_tokens"] = serde_json::json!(max_tokens);
        }

        if let Some(temperature) = self.temperature {
            payload["temperature"] = serde_json::json!(temperature);
        }

        payload
    }

    /// Send a chat/completions request and return the assistant response text.
    ///
    /// Maps known HTTP failures to typed error codes.
    pub async fn chat_completion(&self, system_prompt: &str, user_content: &str) -> Result<String> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let payload = self.build_chat_payload(system_prompt, user_content);

        let response = self
            .http_client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&payload)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    anyhow!(
                        "{}: request timed out after {:?}",
                        LLM_TIMEOUT,
                        self.timeout
                    )
                } else {
                    anyhow!("LLM request failed: {}", e)
                }
            })?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(anyhow!(
                "{}: provider returned {} - check LLM_API_KEY",
                LLM_AUTH_FAILED,
                status
            ));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(anyhow!(
                "{}: provider returned 429 - rate limit exceeded",
                LLM_RATE_LIMITED
            ));
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "LLM request failed with status {}: {}",
                status,
                body
            ));
        }

        let body: Value = response.json().await.map_err(|e| {
            anyhow!(
                "{}: failed to parse LLM response as JSON: {}",
                LLM_INVALID_JSON,
                e
            )
        })?;

        // Extract the assistant message content from OpenAI-compatible response shape:
        // { "choices": [{ "message": { "content": "..." } }] }
        let content = body["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| {
                anyhow!(
                    "{}: unexpected response shape - missing choices[0].message.content",
                    LLM_INVALID_JSON
                )
            })?;

        Ok(content.to_string())
    }

    /// Convenience method: send content to the LLM for structured JSON extraction.
    ///
    /// Wraps `chat_completion` with a JSON-extraction-specific system prompt.
    pub async fn extract_json(
        &self,
        prompt: &str,
        schema_hint: &str,
        content: &str,
    ) -> Result<Value> {
        let system_prompt = format!(
            "{}\n\nExpected JSON schema:\n{}\n\nRespond with ONLY valid JSON, no markdown fences or extra text.",
            prompt, schema_hint
        );

        let raw = self.chat_completion(&system_prompt, content).await?;

        // Try to parse the response as JSON, stripping common wrapper artifacts
        let trimmed = raw.trim();
        let json_str = trimmed
            .strip_prefix("```json")
            .or_else(|| trimmed.strip_prefix("```"))
            .unwrap_or(trimmed);
        let json_str = json_str.strip_suffix("```").unwrap_or(json_str).trim();

        serde_json::from_str(json_str).map_err(|e| {
            anyhow!(
                "{}: LLM response is not valid JSON: {}",
                LLM_INVALID_JSON,
                e,
            )
        })
    }

    /// Return the configured model name (read-only accessor).
    pub fn model(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test-only helper to construct an LlmClient with explicit values.
    fn make_test_client(
        base_url: &str,
        api_key: &str,
        model: &str,
        max_tokens: Option<u32>,
        temperature: Option<f64>,
    ) -> LlmClient {
        LlmClient {
            base_url: base_url.to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            timeout: Duration::from_secs(30),
            max_tokens,
            temperature,
            http_client: reqwest::Client::new(),
        }
    }

    #[test]
    fn test_llm_config_from_env_missing_required_returns_error() {
        // Clear all LLM env vars to ensure a clean state
        std::env::remove_var("LLM_BASE_URL");
        std::env::remove_var("LLM_API_KEY");
        std::env::remove_var("LLM_MODEL");
        let err = LlmClient::from_env().unwrap_err();
        assert!(
            err.to_string().contains("LLM_NOT_CONFIGURED"),
            "Expected error containing LLM_NOT_CONFIGURED, got: {}",
            err
        );
    }

    #[test]
    fn test_openai_compatible_payload_shape() {
        let client = make_test_client(
            "https://api.example.com/v1",
            "sk-test-key-12345",
            "gpt-4",
            Some(1024),
            Some(0.0),
        );

        let payload = client.build_chat_payload(
            "Extract data as JSON",
            "Here is some content to extract from",
        );

        // Must have model field
        assert_eq!(payload["model"], "gpt-4");

        // Must have messages array with system + user
        let messages = payload["messages"]
            .as_array()
            .expect("messages should be array");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "user");

        // System message contains the prompt
        assert!(
            messages[0]["content"]
                .as_str()
                .unwrap()
                .contains("Extract data as JSON"),
            "System message should contain the prompt"
        );

        // User message contains the content
        assert!(
            messages[1]["content"]
                .as_str()
                .unwrap()
                .contains("Here is some content"),
            "User message should contain the content"
        );

        // Must have max_tokens when set
        assert_eq!(payload["max_tokens"], 1024);

        // Must have temperature when set
        assert_eq!(payload["temperature"], 0.0);
    }

    #[test]
    fn test_redact_api_key_in_debug() {
        let client = make_test_client(
            "https://api.example.com/v1",
            "sk-very-secret-key-do-not-leak",
            "gpt-4",
            None,
            None,
        );

        let debug_output = format!("{:?}", client);

        // The actual API key must NOT appear in debug output
        assert!(
            !debug_output.contains("sk-very-secret-key-do-not-leak"),
            "Debug output must not contain the raw API key! Got: {}",
            debug_output
        );

        // But should show a redacted placeholder
        assert!(
            debug_output.contains("sk-v***"),
            "Debug output should contain redacted key prefix, got: {}",
            debug_output
        );
    }

    #[test]
    fn test_redact_key_short_key() {
        assert_eq!(redact_key("abc"), "***");
        assert_eq!(redact_key("abcd"), "***");
        assert_eq!(redact_key(""), "***");
    }

    #[test]
    fn test_redact_key_multibyte_utf8() {
        // Ensures no panic on multi-byte chars at the boundary
        let key = "\u{00e9}\u{00e9}\u{00e9}\u{00e9}\u{00e9}extra"; // e-acute (2-byte) chars
        let redacted = redact_key(key);
        assert!(redacted.ends_with("***"));
        assert!(!redacted.contains("extra"));
    }

    #[test]
    fn test_payload_omits_optional_fields_when_none() {
        let client = make_test_client("https://api.example.com/v1", "sk-test", "gpt-4", None, None);

        let payload = client.build_chat_payload("sys", "user");
        assert!(payload.get("max_tokens").is_none());
        assert!(payload.get("temperature").is_none());
    }
}
