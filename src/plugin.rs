//! `BackendPlugin` impl for OpenAI-compatible chat completions.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use mcpg_backend_llm_shared::chat_config::ResponseFormatMode;
use mcpg_backend_llm_shared::template::Templates;
use mcpg_backend_llm_shared::{
    ChatEngine, ChatProviderAdapter, ProviderError, build_child_tool_defs, compile_validator,
    resolve_api_key,
};
use mcpg_plugin_backend_llm_openai::{OpenAiAdapter, OpenAiVariant};
use mcpg_plugin_protocol::{
    BackendChunkStream, BackendError, BackendHost, BackendPlugin, BackendRequest, BackendResponse,
    PluginManifest, async_trait, firstparty_manifest,
};
use serde_json::Value;
use tracing::{Instrument, debug, info_span, warn};

use crate::config::CompatChatSpec;

pub struct CompatChatPlugin {
    manifest: PluginManifest,
    engines: Arc<RwLock<BTreeMap<String, Arc<ChatEngine>>>>,
}

impl std::fmt::Debug for CompatChatPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompatChatPlugin").finish()
    }
}

impl Default for CompatChatPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl CompatChatPlugin {
    pub fn new() -> Self {
        Self {
            manifest: firstparty_manifest! {
                id: "dev.mcpg.backend.compat.chat",
                name: "OpenAI-Compatible Chat Completions",
                class: Backend,
            },
            engines: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }

    #[doc(hidden)]
    pub fn registered_profile_count(&self) -> usize {
        self.engines.read().unwrap().len()
    }
}

#[async_trait]
impl BackendPlugin for CompatChatPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn kind(&self) -> &str {
        "compat.chat"
    }

    async fn register_profile(
        &self,
        backend_name: &str,
        spec: &Value,
        host: Arc<dyn BackendHost>,
    ) -> Result<(), BackendError> {
        let parsed: CompatChatSpec =
            serde_json::from_value(spec.clone()).map_err(|e| BackendError::InvalidSpec {
                message: format!("compat_chat spec: {e}"),
            })?;
        parsed.validate().map_err(|e| BackendError::InvalidSpec {
            message: e.to_string(),
        })?;

        let api_key = match parsed.api_key.as_ref() {
            Some(r) => resolve_api_key(r)?,
            None => String::new(),
        };
        let connect_timeout = parsed.chat.connect_timeout();

        let adapter = OpenAiAdapter::new(
            OpenAiVariant::Compatible,
            "openai-compatible",
            parsed.base_url.clone(),
            api_key,
            connect_timeout,
        )
        .map_err(|e: ProviderError| BackendError::InvalidSpec {
            message: format!("build compat adapter: {e}"),
        })?;
        let adapter: Arc<dyn ChatProviderAdapter> = Arc::new(adapter);

        let templates = Templates::compile(&parsed.chat.prompt.system, &parsed.chat.prompt.user)
            .map_err(|e| BackendError::InvalidSpec {
                message: format!("template: {e}"),
            })?;

        let (validator, raw_output_schema) = if matches!(
            parsed.chat.response_format.mode,
            ResponseFormatMode::JsonSchema
        ) {
            let schema_value = spec.get("output_schema").cloned();
            if let Some(schema) = schema_value {
                let v = compile_validator(&schema).map_err(|e| BackendError::InvalidSpec {
                    message: e.to_string(),
                })?;
                (Some(v), Some(schema))
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        let child_tool_defs = build_child_tool_defs(&parsed.chat.tools, |_name| None);

        let engine = ChatEngine {
            backend_name: backend_name.to_owned(),
            adapter,
            templates,
            validator,
            raw_output_schema,
            spec: parsed.chat,
            host,
            child_tool_defs,
            child_tool_validators: Vec::new(),
        };

        self.engines
            .write()
            .map_err(|_| BackendError::InvalidSpec {
                message: "engine map poisoned".into(),
            })?
            .insert(backend_name.to_owned(), Arc::new(engine));

        Ok(())
    }

    async fn execute(
        &self,
        backend_name: &str,
        request: BackendRequest,
    ) -> Result<BackendResponse, BackendError> {
        let engine = self
            .engines
            .read()
            .map_err(|_| BackendError::InvalidSpec {
                message: "engine map poisoned".into(),
            })?
            .get(backend_name)
            .cloned()
            .ok_or_else(|| BackendError::ProfileNotFound {
                backend_name: backend_name.to_owned(),
            })?;

        let args: Value = if request.payload.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_slice(&request.payload).map_err(|e| BackendError::InvalidSpec {
                message: format!("execute payload was not JSON: {e}"),
            })?
        };

        // Wrap engine call in a plugin-scoped span
        // so traces from compat chat attribute back to
        // `dev.mcpg.backend.llm.compat` for per-plugin override.
        let span = info_span!(
            "compat_chat_execute",
            plugin_id = "dev.mcpg.backend.llm.compat",
            binding = %backend_name,
            model = %engine.spec.model,
        );
        let started = std::time::Instant::now();
        let result = engine
            .execute(&args, &request.request_id, request.session_id.as_deref())
            .instrument(span)
            .await;
        let elapsed = started.elapsed();

        metrics::counter!(
            "mcpg_llm_calls_total",
            "binding" => backend_name.to_owned(),
            "provider" => engine.adapter.label().to_string(),
            "model" => engine.spec.model.clone(),
            "status" => if result.is_ok() { "ok" } else { "error" },
        )
        .increment(1);
        metrics::histogram!(
            "mcpg_llm_call_overall_seconds",
            "binding" => backend_name.to_owned(),
            "provider" => engine.adapter.label().to_string(),
            "model" => engine.spec.model.clone(),
        )
        .record(elapsed.as_secs_f64());

        match &result {
            Ok(_) => debug!(
                binding = %backend_name,
                model = %engine.spec.model,
                elapsed_ms = %elapsed.as_millis(),
                "compat chat call succeeded"
            ),
            Err(e) => warn!(
                binding = %backend_name,
                model = %engine.spec.model,
                error = %e,
                "compat chat call failed"
            ),
        }

        let value = result?;
        let payload = serde_json::to_vec(&value).map_err(|e| BackendError::Transport {
            message: format!("serialize response: {e}"),
        })?;
        Ok(BackendResponse {
            payload,
            truncated: false,
        })
    }

    async fn execute_streaming(
        &self,
        backend_name: &str,
        request: BackendRequest,
    ) -> Result<BackendChunkStream, BackendError> {
        let engine = self
            .engines
            .read()
            .map_err(|_| BackendError::InvalidSpec {
                message: "engine map poisoned".into(),
            })?
            .get(backend_name)
            .cloned()
            .ok_or_else(|| BackendError::ProfileNotFound {
                backend_name: backend_name.to_owned(),
            })?;

        let args: Value = if request.payload.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_slice(&request.payload).map_err(|e| BackendError::InvalidSpec {
                message: format!("execute payload was not JSON: {e}"),
            })?
        };

        Ok(engine.execute_streaming(args, request.request_id, request.session_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcpg_plugin_protocol::noop_backend_host;

    #[test]
    fn plugin_kind_and_manifest() {
        let p = CompatChatPlugin::new();
        assert_eq!(p.kind(), "compat.chat");
        assert_eq!(p.manifest().id, "dev.mcpg.backend.compat.chat");
    }

    #[tokio::test]
    async fn register_minimal_spec_succeeds() {
        let plugin = CompatChatPlugin::new();
        plugin
            .register_profile(
                "compat",
                &serde_json::json!({
                    "base_url": "http://localhost:8000/v1",
                    "model": "llama3",
                    "prompt": { "system": "x", "user": "{{ input.text }}" }
                }),
                noop_backend_host(),
            )
            .await
            .unwrap();
        assert_eq!(plugin.registered_profile_count(), 1);
    }

    #[tokio::test]
    async fn register_rejects_missing_base_url() {
        let plugin = CompatChatPlugin::new();
        let err = plugin
            .register_profile(
                "c",
                &serde_json::json!({
                    "base_url": "",
                    "model": "x",
                    "prompt": { "system": "x", "user": "y" }
                }),
                noop_backend_host(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::InvalidSpec { .. }));
    }

    #[tokio::test]
    async fn register_with_api_key_succeeds() {
        let plugin = CompatChatPlugin::new();
        plugin
            .register_profile(
                "groq",
                &serde_json::json!({
                    "base_url": "https://api.groq.com/openai/v1",
                    "api_key": "k",
                    "model": "llama-3.3-70b-versatile",
                    "prompt": { "system": "x", "user": "{{ input.text }}" }
                }),
                noop_backend_host(),
            )
            .await
            .unwrap();
        assert_eq!(plugin.registered_profile_count(), 1);
    }
}
