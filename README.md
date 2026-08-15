# OpenAI-Compatible Backends — `dev.mcpg.backend.llm.compat`

> class `backend` · `native` · package `mcpg-plugin-backend-llm-compat` · artifact `libmcpg_plugin_backend_llm_compat.so` · Apache-2.0

Exposes any endpoint that speaks the OpenAI `/chat/completions` and
`/embeddings` ABIs — but is not OpenAI — as MCP capabilities: vLLM, LocalAI,
Together, Groq, OpenRouter, llama.cpp's OpenAI server, Vertex AI's OpenAI
compatibility surface, and anything else that implements the same wire format.
The operator supplies the endpoint URL; the API key is optional, because many
self-hosted servers accept any key or none. Reach for it to put a self-hosted or
third-party model behind the same governed, budgeted, audited MCP surface as a
hosted provider, without writing a plugin.

## What it does
- Registers two backend entities under one cdylib. Each self-describes its
  `BackendPlugin::kind()` at load time, so the gateway dispatches each binding
  to the right one.
- Reuses the wire-format adapter from `mcpg-plugin-backend-llm-openai` in its
  `Compatible` variant, so behaviour tracks the OpenAI plugin exactly: same
  request encoding, same streaming decode, same error-status mapping.
- Omits the `Authorization` header entirely when `api_key` is not declared, so
  endpoints with no auth work unmodified.
- Renders `prompt.system` and `prompt.user` as MiniJinja templates over
  `input.*` (the caller's tool arguments) and `meta.*` (`backend_name`,
  `request_id`, `session_id`, `timestamp_iso8601`).
- Runs a bounded agentic loop over child MCP tools named in `tools.allowed`,
  refusing any call the model invents outside that list before it leaves the
  plugin.
- Streams incremental chat tokens over SSE and validates structured output
  binding-side, so an endpoint with weak or missing native JSON-schema support
  still gets a hard contract.
- Splits large embedding batches across parallel calls.
- Retries rate-limit, 5xx and network failures with exponential backoff, and
  enforces per-binding token and daily-USD budget caps before spending.
- Declares the `network_outbound` capability — required in every mode, since
  every call is an outbound request to the configured endpoint.

| `backend.kind` | Registry kind | Entity id | Surface |
|---|---|---|---|
| `compat_chat` | `compat.chat` | `dev.mcpg.backend.compat.chat` | chat completions |
| `compat_embedding` | `compat.embedding` | `dev.mcpg.backend.compat.embedding` | embeddings |

## Configuration

Load the artifact once from the flat top-level `plugins:` list, then declare one
binding per capability under `mcp.capabilities.tools[]` (or `.prompts[]` /
`.resources[]`) with `backend.kind: compat_chat` or `compat_embedding`.
Everything else inside the `backend:` block is the plugin's own spec, forwarded
verbatim and validated by the plugin at boot — an invalid value fails gateway
startup, not the first call.

`base_url` is the one field this plugin cannot default: it has no canonical
endpoint. Give it the URL *up to but not including* `/chat/completions`, which
the adapter appends itself.

```yaml
plugins:
  - id: dev.mcpg.backend.llm.compat
    class: backend
    source:
      oci: ghcr.io/mcpg-dev/source-code/plugins/backend-llm-compat:protocol-1

mcp:
  capabilities:
    tools:
      - name: local.chat
        description: Chat with the in-cluster vLLM deployment.
        input_schema:
          type: object
          properties:
            question: { type: string }
          required: [question]
        backend:
          kind: compat_chat
          base_url: "http://vllm.internal.svc:8000/v1"   # required
          # api_key omitted — vLLM accepts unauthenticated requests here
          model: meta-llama/Llama-3.1-8B-Instruct
          prompt:
            system: You are a concise assistant.
            user: "{{ input.question }}"
          response_format:
            mode: text
          sampling:
            temperature: 0.2
            max_completion_tokens: 1024
```

### Provider fields

| Field | Type | Default | Description |
|---|---|---|---|
| `base_url` | string | *(required)* | Endpoint base URL up to but not including `/chat/completions` (or `/embeddings`). An empty or whitespace value is refused at boot. |
| `api_key` | string | *(optional)* | Sent as `Authorization: Bearer …` when present and non-empty; the header is omitted entirely otherwise. Supply `${env.NAME}` or a `scheme://` URI bound to a `secret_provider` plugin (for example `vault://secret/groq#key`); the gateway substitutes the literal value at config load. |

### Chat execution fields (`compat_chat`)

Shared with every other MCPG chat binding, so switching providers means changing
`kind`, `base_url` and `model` — not relearning the schema.

| Field | Type | Default | Description |
|---|---|---|---|
| `model` | string | *(required)* | Model id the endpoint understands. |
| `prompt.system` | string | *(required)* | System-prompt template. Must be non-empty after trimming. |
| `prompt.user` | string | *(required)* | User-prompt template. Must be non-empty after trimming. |
| `prompt.image_inputs` | string[] | `[]` | Argument names carrying image content (URL, `data:` URL, raw base64, `mcpg-resource://` URI, or an explicit object). An array value fans out to several parts. |
| `prompt.audio_inputs` | string[] | `[]` | Argument names carrying audio; base64 sources become `input_audio` parts. |
| `prompt.file_inputs` | string[] | `[]` | Argument names carrying documents; object values may set `mime_type` and `filename`. |
| `timeout_ms` | integer | `60000` | Per-iteration wall-clock budget upstream, retries included. |
| `connect_timeout_ms` | integer | `5000` | TCP connect timeout, kept separate so a slow-but-connected upstream is not killed early. |
| `sampling.temperature` | number | *(unset)* | Passed through when set. |
| `sampling.top_p` | number | *(unset)* | Passed through when set. |
| `sampling.max_completion_tokens` | integer | *(unset)* | Per-iteration output cap. |
| `sampling.seed` | integer | *(unset)* | Passed through when set. |
| `response_format.mode` | `json_schema` \| `text` | `json_schema` | `text` wraps the reply as `{"text": "…"}` and skips validation. Prefer `text` on endpoints with no native JSON-schema support. |
| `response_format.strict` | boolean | `true` | Requests provider-side strictness where available; binding-side validation runs either way. |
| `response_format.on_mismatch` | `error` \| `retry_once` \| `return_raw` | `error` | `return_raw` is legal only with `mode: text`. |
| `tools.allowed` | string[] | `[]` | Names of other bindings in this gateway the model may call. Empty means single-shot. |
| `tools.max_iterations` | integer | `1` when `allowed` is empty, else `5` | Maximum model round-trips. Values above `50` are refused at boot. |
| `tools.tool_choice` | `auto` \| `required` \| `none` | `auto` | Provider-level tool-choice hint. |
| `tools.tool_result_max_bytes` | integer | `16384` | Each child result is truncated to this before re-entering the conversation. |
| `tools.on_iteration_exhausted` | `error` \| `return_partial` | `error` | What happens when the loop runs out of iterations. |
| `retry.max_attempts` | integer | `3` | Attempts per upstream call. |
| `retry.initial_backoff_ms` | integer | `500` | First backoff; must not exceed `max_backoff_ms`. |
| `retry.max_backoff_ms` | integer | `8000` | Backoff ceiling. |
| `retry.retry_on` | list of `rate_limited` \| `server` \| `network` | all three | Failure classes worth retrying. |
| `guardrails.max_output_tokens_per_iteration` | integer | *(unset)* | Hard cap that overrides `sampling.max_completion_tokens`. |
| `cache.enabled` | boolean | `false` | Opt-in response cache. Refused at boot together with a non-empty `tools.allowed`. |
| `cache.ttl_seconds` | integer | `3600000` | Per-entry TTL, in seconds. |
| `budget.tokens_per_call_cap` | integer | `0` (uncapped) | Total input + output tokens across all loop iterations of one call. Checked between iterations, never on the first. |
| `budget.usd_daily_cap` | number | `0` (uncapped) | Aggregate spend for this binding per UTC day, checked before each call. Inert unless the model appears in the bundled rate card. |
| `output_schema` | object | *(unset)* | JSON Schema the reply must satisfy under `mode: json_schema`. Read out of this `backend:` block, not the binding-level field. |

### Embedding fields (`compat_embedding`)

| Field | Type | Default | Description |
|---|---|---|---|
| `model` | string | *(required)* | Embedding model id the endpoint understands. |
| `dimensions` | integer | *(unset)* | Requests reduced vectors; ignored by endpoints that do not support it. |
| `max_batch_size` | integer | provider cap | Per-call batch size. Larger inputs split into parallel calls. |
| `timeout_ms` | integer | `10000` | Per-call timeout. |
| `connect_timeout_ms` | integer | `5000` | TCP connect timeout. |
| `retry.max_attempts` | integer | `3` | Attempts per upstream call. |
| `retry.initial_backoff_ms` | integer | `200` | First backoff. |
| `retry.max_backoff_ms` | integer | `2000` | Backoff ceiling. |
| `cache.enabled` | boolean | `false` | Opt-in; `text → vector` is deterministic, so caching is sound. |
| `cache.ttl_seconds` | integer | `86400` | Per-entry TTL, in seconds. |

## Operations

A `compat_embedding` binding takes `input` — a single string or an array of
strings — and returns `{embeddings, dimensions, usage}`, with `embeddings`
carrying exactly one entry per input.

```yaml
      - name: local.embed
        description: Embed passages against the local inference server.
        backend:
          kind: compat_embedding
          base_url: "http://vllm.internal.svc:8000/v1"
          model: BAAI/bge-large-en-v1.5
          max_batch_size: 64
          cache: { enabled: true }
```

## Response envelope

Chat bindings under `response_format.mode: json_schema` return the validated
object as-is; a reply that is not valid JSON or does not satisfy the schema
either fails the call or earns one corrective round-trip, per
`response_format.on_mismatch`. Under `mode: text` they return `{"text": "…"}` and
skip validation entirely. Because binding-side validation runs regardless of what the
endpoint supports, `mode: json_schema` still gives a hard contract on servers
whose native structured-output support is partial — set
`on_mismatch: retry_once` so the model gets one corrective turn.

## Security

- The API key, when present, is held in a redacting wrapper — `Debug` renders
  `***`, so it cannot leak through logs or error strings. If you *do* declare
  `api_key`, a value that resolves to empty is rejected at boot rather than
  silently sending unauthenticated requests.
- `base_url` is operator config, never a caller argument, so a caller cannot
  redirect the binding at another host.
- Prompt templates can reference only `input.*` and `meta.*`. There is no
  filesystem loader, no env-var lookup, and the `debug` filter is removed, so a
  template cannot dump gateway state or exfiltrate the context. Undefined
  variables fail loudly instead of rendering empty.
- `tools.allowed` is an explicit allowlist enforced inside the plugin: a tool
  call the model invents that is not on the list never leaves the plugin. The
  gateway refuses a child call that targets the initiating binding itself and
  caps child-invocation depth at 8, on top of `tools.max_iterations`.
- Child tool calls carry no caller identity, and `cred://` credential threading
  is unsupported on that path. They are ungated unless you turn on
  `governance.child_invoke.enforce_gates`, which makes each child call run the
  same policy chain, trust floor, CEL `allow_if` gate and tool-gate chain a
  direct `tools/call` runs.

## Observability

Chat calls emit `mcpg_llm_calls_total` and `mcpg_llm_call_overall_seconds`,
labelled with `binding`, `provider` (`openai-compatible`), `model` and `status`.
The shared engine adds `mcpg_llm_call_duration_seconds`, `mcpg_llm_iterations`,
`mcpg_llm_retries_total`, `mcpg_llm_tool_calls_total`,
`mcpg_llm_schema_validation_errors_total`, `mcpg_llm_cache_hits_total` /
`mcpg_llm_cache_misses_total` and `mcpg_llm_budget_refusals_total`. Embedding
calls emit `mcpg_embedding_call_seconds` and `mcpg_embedding_inputs_total`.

Cost and token metrics depend on the rate card vendored in
`mcpg-backend-llm-shared`; models it does not list are not priced, so spend
counters stay silent for them rather than reporting a misleading zero.

## MCP surfaces & composition

### As a child tool

A `compat_chat` binding can appear in another chat binding's `tools.allowed`,
which is the usual way to route cheap or private traffic to a self-hosted model
while a hosted model handles the rest — no gateway-side orchestration code.

```yaml
        backend:
          kind: openai_chat
          api_key: "${env.OPENAI_API_KEY}"
          model: gpt-4o-mini
          prompt:
            system: Delegate anything containing customer data to `local.chat`.
            user: "{{ input.question }}"
          tools:
            allowed: [local.chat]   # a binding backed by compat_chat
```

### Schemas & annotations

The binding-level `input_schema` is what clients see in `tools/list` and what
the gateway validates arguments against. The `output_schema` *inside* the
`backend:` block is what a chat binding enforces on the model's reply; declare
the binding-level `output_schema` too when you want clients to see the
contract. Mark bindings that only read as side-effect-free:

```yaml
        annotations: { read_only: true, open_world: true }
```

## Build
Default feature set is OFF (avoids `mcpg_plugin_register` linker
collisions in the workspace build); opt in to the cdylib:

```bash
cargo build -p mcpg-plugin-backend-llm-compat --features cdylib-export --release   # → target/release/libmcpg_plugin_backend_llm_compat.so
```

## Sign & load (production)
Sign the artifact, pin/verify via the entry's `signature:` block, and honour
revocations. See <https://mcpg.dev/docs/security/plugin-security>.

## See also
- Backend binding reference: <https://mcpg.dev/docs/reference/backends>
- Full gateway config schema: <https://mcpg.dev/docs/reference/configuration>
- The wire adapter this plugin reuses: `libs/plugins/backend/llms/openai`
- Provider-agnostic engines and shared config types: `libs/plugins/backend/llms/shared`
