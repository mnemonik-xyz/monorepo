//! Universal LLM provider abstraction.
//!
//! Supports OpenAI-compatible providers (ollama, groq, openrouter, together,
//! cerebras, openai) and the Anthropic Messages API via a single `LlmClient`.

use serde_json::Value;

/// Known LLM provider identifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmProvider {
    Ollama,
    Groq,
    OpenRouter,
    Together,
    Cerebras,
    OpenAi,
    Anthropic,
}

impl LlmProvider {
    /// Parse a provider string (case-insensitive).
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "ollama" => Ok(Self::Ollama),
            "groq" => Ok(Self::Groq),
            "openrouter" => Ok(Self::OpenRouter),
            "together" => Ok(Self::Together),
            "cerebras" => Ok(Self::Cerebras),
            "openai" => Ok(Self::OpenAi),
            "anthropic" => Ok(Self::Anthropic),
            other => Err(format!(
                "unknown LLM_PROVIDER '{other}'; expected one of: ollama, groq, openrouter, together, cerebras, openai, anthropic"
            )),
        }
    }

    /// Default model for this provider.
    pub fn default_model(&self) -> &'static str {
        match self {
            Self::Ollama => "qwen2.5:3b",
            Self::Groq => "llama-3.1-8b-instant",
            Self::OpenRouter => "meta-llama/llama-3.1-8b-instruct",
            Self::Together => "meta-llama/Meta-Llama-3.1-8B-Instruct-Turbo",
            Self::Cerebras => "llama3.1-8b",
            Self::OpenAi => "gpt-4o-mini",
            Self::Anthropic => "claude-sonnet-4-20250514",
        }
    }

    /// Default base URL for this provider.
    pub fn default_base_url(&self) -> &'static str {
        match self {
            Self::Ollama => "http://localhost:11434",
            Self::Groq => "https://api.groq.com/openai",
            Self::OpenRouter => "https://openrouter.ai/api",
            Self::Together => "https://api.together.xyz",
            Self::Cerebras => "https://api.cerebras.ai",
            Self::OpenAi => "https://api.openai.com",
            Self::Anthropic => "https://api.anthropic.com",
        }
    }

    /// Whether this provider uses the Anthropic Messages API format.
    pub fn is_anthropic(&self) -> bool {
        matches!(self, Self::Anthropic)
    }

    /// Whether this provider requires an API key.
    pub fn requires_api_key(&self) -> bool {
        !matches!(self, Self::Ollama)
    }
}

/// Universal LLM client that dispatches to OpenAI-compatible or Anthropic APIs.
#[derive(Debug, Clone)]
pub struct LlmClient {
    pub provider: LlmProvider,
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub max_tokens: u32,
}

impl LlmClient {
    /// Create a new LlmClient, validating configuration.
    pub fn new(
        provider: &str,
        api_key: &str,
        model: &str,
        api_url: &str,
        max_tokens: u32,
    ) -> Result<Self, String> {
        let prov = LlmProvider::from_str(provider)?;

        if prov.requires_api_key() && api_key.is_empty() {
            return Err(format!(
                "LLM_API_KEY is required for provider '{provider}'"
            ));
        }

        let resolved_model = if model.is_empty() {
            prov.default_model().to_string()
        } else {
            model.to_string()
        };

        let resolved_url = if api_url.is_empty() {
            prov.default_base_url().to_string()
        } else {
            api_url.to_string()
        };

        Ok(Self {
            provider: prov,
            api_key: api_key.to_string(),
            model: resolved_model,
            base_url: resolved_url.trim_end_matches('/').to_string(),
            max_tokens,
        })
    }

    /// Send a chat request and return the response text.
    pub async fn chat(
        &self,
        http_client: &reqwest::Client,
        system_prompt: &str,
        user_message: &str,
    ) -> Result<String, String> {
        if self.provider.is_anthropic() {
            self.chat_anthropic(http_client, system_prompt, user_message)
                .await
        } else {
            self.chat_openai_compat(http_client, system_prompt, user_message)
                .await
        }
    }

    /// Build the request body for OpenAI-compatible providers.
    pub fn build_openai_body(&self, system_prompt: &str, user_message: &str) -> Value {
        serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_message},
            ],
            "max_tokens": self.max_tokens,
            "temperature": 0.3,
            "stream": false,
        })
    }

    /// Build the request body for the Anthropic Messages API.
    pub fn build_anthropic_body(&self, system_prompt: &str, user_message: &str) -> Value {
        serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "user", "content": user_message},
            ],
            "max_tokens": self.max_tokens,
            "temperature": 0.3,
            "system": system_prompt,
        })
    }

    /// POST to an OpenAI-compatible `/v1/chat/completions` endpoint.
    async fn chat_openai_compat(
        &self,
        http_client: &reqwest::Client,
        system_prompt: &str,
        user_message: &str,
    ) -> Result<String, String> {
        let url = format!("{}/v1/chat/completions", self.base_url);
        let body = self.build_openai_body(system_prompt, user_message);

        let mut req = http_client.post(&url).json(&body);
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }

        let resp = req.send().await.map_err(|e| {
            format!("LLM request failed ({}): {e}", self.provider_name())
        })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(format!(
                "LLM returned status {status} ({}): {body_text}",
                self.provider_name()
            ));
        }

        let json: Value = resp.json().await.map_err(|e| {
            format!("failed to parse LLM response ({}): {e}", self.provider_name())
        })?;

        // OpenAI format: choices[0].message.content
        json["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| {
                format!(
                    "unexpected LLM response structure ({}): {}",
                    self.provider_name(),
                    serde_json::to_string(&json).unwrap_or_default()
                )
            })
    }

    /// POST to the Anthropic `/v1/messages` endpoint.
    async fn chat_anthropic(
        &self,
        http_client: &reqwest::Client,
        system_prompt: &str,
        user_message: &str,
    ) -> Result<String, String> {
        let url = format!("{}/v1/messages", self.base_url);
        let body = self.build_anthropic_body(system_prompt, user_message);

        let resp = http_client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("LLM request failed (anthropic): {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(format!(
                "LLM returned status {status} (anthropic): {body_text}"
            ));
        }

        let json: Value = resp.json().await.map_err(|e| {
            format!("failed to parse LLM response (anthropic): {e}")
        })?;

        // Anthropic format: content[0].text
        json["content"][0]["text"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| {
                format!(
                    "unexpected LLM response structure (anthropic): {}",
                    serde_json::to_string(&json).unwrap_or_default()
                )
            })
    }

    fn provider_name(&self) -> &str {
        match self.provider {
            LlmProvider::Ollama => "ollama",
            LlmProvider::Groq => "groq",
            LlmProvider::OpenRouter => "openrouter",
            LlmProvider::Together => "together",
            LlmProvider::Cerebras => "cerebras",
            LlmProvider::OpenAi => "openai",
            LlmProvider::Anthropic => "anthropic",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- LlmProvider parsing --

    #[test]
    fn parse_all_providers() {
        assert_eq!(LlmProvider::from_str("ollama").unwrap(), LlmProvider::Ollama);
        assert_eq!(LlmProvider::from_str("groq").unwrap(), LlmProvider::Groq);
        assert_eq!(LlmProvider::from_str("openrouter").unwrap(), LlmProvider::OpenRouter);
        assert_eq!(LlmProvider::from_str("together").unwrap(), LlmProvider::Together);
        assert_eq!(LlmProvider::from_str("cerebras").unwrap(), LlmProvider::Cerebras);
        assert_eq!(LlmProvider::from_str("openai").unwrap(), LlmProvider::OpenAi);
        assert_eq!(LlmProvider::from_str("anthropic").unwrap(), LlmProvider::Anthropic);
    }

    #[test]
    fn parse_case_insensitive() {
        assert_eq!(LlmProvider::from_str("GROQ").unwrap(), LlmProvider::Groq);
        assert_eq!(LlmProvider::from_str("Anthropic").unwrap(), LlmProvider::Anthropic);
    }

    #[test]
    fn parse_unknown_provider_errors() {
        let err = LlmProvider::from_str("gpt4all").unwrap_err();
        assert!(err.contains("unknown LLM_PROVIDER"));
        assert!(err.contains("gpt4all"));
    }

    // -- LlmClient construction --

    #[test]
    fn new_ollama_no_api_key_ok() {
        let client = LlmClient::new("ollama", "", "", "", 512).unwrap();
        assert_eq!(client.provider, LlmProvider::Ollama);
        assert_eq!(client.model, "qwen2.5:3b");
        assert_eq!(client.base_url, "http://localhost:11434");
        assert_eq!(client.max_tokens, 512);
    }

    #[test]
    fn new_groq_requires_api_key() {
        let err = LlmClient::new("groq", "", "", "", 512).unwrap_err();
        assert!(err.contains("LLM_API_KEY is required"));
    }

    #[test]
    fn new_anthropic_requires_api_key() {
        let err = LlmClient::new("anthropic", "", "", "", 512).unwrap_err();
        assert!(err.contains("LLM_API_KEY is required"));
    }

    #[test]
    fn new_custom_model_and_url() {
        let client = LlmClient::new("openai", "sk-test", "gpt-4", "https://custom.api.com/", 1024).unwrap();
        assert_eq!(client.model, "gpt-4");
        assert_eq!(client.base_url, "https://custom.api.com");
        assert_eq!(client.max_tokens, 1024);
    }

    #[test]
    fn new_defaults_per_provider() {
        let client = LlmClient::new("together", "key123", "", "", 512).unwrap();
        assert_eq!(client.model, "meta-llama/Meta-Llama-3.1-8B-Instruct-Turbo");
        assert_eq!(client.base_url, "https://api.together.xyz");
    }

    // -- Request body building --

    #[test]
    fn openai_body_structure() {
        let client = LlmClient::new("groq", "key", "", "", 256).unwrap();
        let body = client.build_openai_body("You are helpful.", "Hello");

        assert_eq!(body["model"], "llama-3.1-8b-instant");
        assert_eq!(body["max_tokens"], 256);
        assert_eq!(body["temperature"], 0.3);
        assert_eq!(body["stream"], false);

        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "You are helpful.");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"], "Hello");
    }

    #[test]
    fn anthropic_body_structure() {
        let client = LlmClient::new("anthropic", "key", "", "", 512).unwrap();
        let body = client.build_anthropic_body("You are helpful.", "Hello");

        assert_eq!(body["model"], "claude-sonnet-4-20250514");
        assert_eq!(body["max_tokens"], 512);
        assert_eq!(body["temperature"], 0.3);
        assert_eq!(body["system"], "You are helpful.");

        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "Hello");

        // Anthropic body must NOT have "stream" key (default is non-streaming)
        assert!(body.get("stream").is_none());
    }

    #[test]
    fn openai_body_no_system_in_top_level() {
        let client = LlmClient::new("openai", "key", "", "", 512).unwrap();
        let body = client.build_openai_body("sys", "usr");
        // system prompt goes into messages array, not top-level "system" field
        assert!(body.get("system").is_none());
    }

    #[test]
    fn anthropic_body_no_stream_key() {
        let client = LlmClient::new("anthropic", "key", "", "", 512).unwrap();
        let body = client.build_anthropic_body("sys", "usr");
        assert!(body.get("stream").is_none());
    }
}
