//! Operator-facing config for the OpenAI-compatible chat binding.

use mcpg_backend_llm_shared::{ApiKeyRef, ChatExecutionSpec, ConfigError, EmbeddingExecutionSpec};
use serde::{Deserialize, Serialize};

/// Spec for `binding_type: compat_chat`.
///
/// `base_url` is required — this binding has no provider-specific
/// default URL. `api_key` is optional because many self-hosted
/// endpoints (vLLM, LocalAI, llama.cpp's OpenAI server) accept any
/// or no key. When omitted, the adapter sends no `Authorization`
/// header.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatChatSpec {
    /// Required. Base URL up to (but not including) `/chat/completions`.
    /// The adapter appends `/chat/completions` itself.
    pub base_url: String,

    /// Optional. Some endpoints accept any/no key.
    #[serde(default)]
    pub api_key: Option<ApiKeyRef>,

    #[serde(flatten)]
    pub chat: ChatExecutionSpec,
}

impl CompatChatSpec {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.base_url.trim().is_empty() {
            return Err(ConfigError::InvalidSpec(
                "compat_chat: base_url is required (operator must declare the endpoint URL up to \
                 but not including `/chat/completions`)"
                    .into(),
            ));
        }
        self.chat.validate()
    }
}

/// Spec for `binding_type: compat_embedding`. Same shape as
/// `CompatChatSpec` — `base_url` required, `api_key` optional, plus
/// the provider-agnostic embedding fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatEmbeddingSpec {
    pub base_url: String,
    #[serde(default)]
    pub api_key: Option<ApiKeyRef>,
    #[serde(flatten)]
    pub embedding: EmbeddingExecutionSpec,
}

impl CompatEmbeddingSpec {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.base_url.trim().is_empty() {
            return Err(ConfigError::InvalidSpec(
                "compat_embedding: base_url is required".into(),
            ));
        }
        self.embedding.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcpg_backend_llm_shared::PromptSpec;
    use serde_json::json;

    fn minimal() -> ChatExecutionSpec {
        ChatExecutionSpec {
            model: "llama3-70b".into(),
            timeout_ms: 30_000,
            connect_timeout_ms: 5_000,
            prompt: PromptSpec {
                system: "you are helpful".into(),
                user: "{{ input.text }}".into(),
                ..Default::default()
            },
            sampling: Default::default(),
            response_format: Default::default(),
            tools: Default::default(),
            streaming: Default::default(),
            retry: Default::default(),
            guardrails: Default::default(),
            cache: Default::default(),
            budget: Default::default(),
        }
    }

    #[test]
    fn requires_base_url() {
        let s = CompatChatSpec {
            base_url: "  ".into(),
            api_key: None,
            chat: minimal(),
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn accepts_no_api_key() {
        let s = CompatChatSpec {
            base_url: "http://localhost:8000/v1".into(),
            api_key: None,
            chat: minimal(),
        };
        s.validate().unwrap();
    }

    #[test]
    fn json_round_trip_minimal() {
        let json = json!({
            "base_url": "https://api.together.xyz/v1",
            "model": "meta-llama/Llama-3-70b-chat-hf",
            "prompt": { "system": "x", "user": "y" }
        });
        let s: CompatChatSpec = serde_json::from_value(json).unwrap();
        assert!(s.api_key.is_none());
        s.validate().unwrap();
    }

    #[test]
    fn json_round_trip_with_key() {
        let json = json!({
            "base_url": "https://api.groq.com/openai/v1",
            "api_key": "k",
            "model": "llama-3.3-70b-versatile",
            "prompt": { "system": "x", "user": "y" }
        });
        let s: CompatChatSpec = serde_json::from_value(json).unwrap();
        assert!(s.api_key.is_some());
        s.validate().unwrap();
    }
}
