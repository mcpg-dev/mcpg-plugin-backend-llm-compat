//! # mcpg-plugin-backend-llm-compat
//!
//! OpenAI-compatible chat-completion binding plugin for MCPG. Ships
//! [`CompatChatPlugin`] (`kind: "compat.chat"`).
//!
//! Targets endpoints that speak the OpenAI `/chat/completions` ABI
//! but are not OpenAI itself: vLLM, LocalAI, Together, Groq,
//! OpenRouter, llama.cpp's OpenAI server, Vertex AI's OpenAI
//! compatibility surface, and others. Reuses `OpenAiAdapter` (with
//! [`mcpg_plugin_backend_llm_openai::OpenAiVariant::Compatible`])
//! from the sibling first-party crate.
//!
//! Operators must declare `base_url`. `api_key` is optional — many
//! self-hosted endpoints accept any/no key, and the adapter omits
//! the `Authorization` header when the resolved key is empty.

/// cdylib sync bridge + `declare_plugin!` export (backend-plugin-migration).
/// Additive: the gateway keeps using the static `new()` path. The
/// `mcpg_plugin_register` FFI symbol is gated behind the `cdylib-export`
/// feature inside the macro expansion. Public so the wrapper types +
/// macro-generated entity modules are part of the crate's public surface
/// (mirrors openai / the nats / kafka pilots, which keep their bridges at
/// crate root) — this also keeps the wrappers from tripping `dead_code`
/// on the default rlib build where neither `cdylib-export` nor
/// `static-firstparty` references them.
pub mod cdylib;
mod config;
mod embedding_plugin;
mod plugin;

pub use config::{CompatChatSpec, CompatEmbeddingSpec};
pub use embedding_plugin::CompatEmbeddingPlugin;
pub use plugin::CompatChatPlugin;
