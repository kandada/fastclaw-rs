// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Anthropic gateway adapter (Claude / MiniMax / DeepSeek-anthropic …).
//!
//! Extended thinking follows the Python fastclaw strategy:
//! - `adaptive` by default (Claude 4.6+ / MiniMax),
//! - falls back to `enabled { budget_tokens }` (Claude 4.5-),
//! - then `none`, when the API explicitly rejects the current mode.
//!
//! The working mode is cached per model for the process lifetime.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};

use anthropic_client_rs::{
    AnthropicAsyncClient, ChatMessage, ContentBlock, ImageSource, ThinkingConfig, Tool,
};
use async_trait::async_trait;
use futures::StreamExt;

use super::provider::{
    LlmProvider, LlmRequest, LlmResponse, LlmSink, Message, ThinkingPref, ToolCall, ToolSchema,
};
use crate::error::{Error, Result};

const DEFAULT_MAX_TOKENS: u64 = 4096;
const DEFAULT_BUDGET_TOKENS: u64 = 30000;

/// Per-model cached thinking mode (process-lifetime, like Python's in-process cache).
fn thinking_mode_cache() -> &'static Mutex<HashMap<String, String>> {
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Build the ordered list of thinking configs to try for a preference.
pub fn thinking_candidates(pref: ThinkingPref) -> Vec<Option<ThinkingConfig>> {
    match pref {
        ThinkingPref::Disabled => vec![None],
        ThinkingPref::Adaptive => vec![
            Some(ThinkingConfig::adaptive()),
            Some(ThinkingConfig::enabled(DEFAULT_BUDGET_TOKENS)),
            None,
        ],
        ThinkingPref::Enabled(budget) => {
            vec![Some(ThinkingConfig::enabled(budget.max(1024))), None]
        }
    }
}

/// A short label for a thinking config (for the rejection detector + cache).
fn thinking_label(cfg: &Option<ThinkingConfig>) -> &'static str {
    match cfg {
        None => "none",
        Some(ThinkingConfig::Adaptive { .. }) => "adaptive",
        Some(ThinkingConfig::Enabled { .. }) => "enabled",
        Some(ThinkingConfig::Disabled) => "none",
    }
}

/// Strictly detect whether the API rejected the request *because* the given
/// thinking mode is unsupported (mirrors Python `is_thinking_mode_rejected`).
pub fn is_thinking_rejected(err: &str, mode: &str) -> bool {
    let msg = err.to_lowercase();
    // Official Anthropic precise format: `'"thinking.type.enabled" is not supported'`.
    let needle = format!("\"thinking.type.{mode}\"");
    if msg.contains(&needle) {
        return true;
    }
    // If the error names another specific mode, the current mode is NOT rejected.
    for other in ["adaptive", "enabled"] {
        if other != mode && msg.contains(&format!("\"thinking.type.{other}\"")) {
            return false;
        }
    }
    // Third-party proxies rejecting the whole field.
    if msg.contains("unknown") && msg.contains("thinking") {
        return true;
    }
    // Generic fallback.
    msg.contains("not supported") && msg.contains("thinking")
}

/// Convert a normalized message into an Anthropic `ChatMessage`.
///
/// Assistant messages carrying `reasoning_content` (and an optional
/// `thinking_signature`) are rebuilt as a `thinking` block so multi-turn
/// conversations pass the signature back — required by Anthropic's protocol.
pub fn to_anthropic_message(msg: &Message) -> ChatMessage {
    let content = msg.content.clone();
    match msg.role.as_str() {
        "assistant" => {
            let has_reasoning = msg
                .reasoning_content
                .as_ref()
                .map(|r| !r.is_empty())
                .unwrap_or(false);
            if has_reasoning {
                let mut blocks: Vec<ContentBlock> = Vec::new();
                if let Some(r) = &msg.reasoning_content {
                    if !r.is_empty() {
                        blocks.push(ContentBlock::Thinking {
                            thinking: r.clone(),
                            signature: msg.thinking_signature.clone().unwrap_or_default(),
                        });
                    }
                }
                if !content.is_empty() {
                    blocks.push(ContentBlock::Text {
                        text: content.clone(),
                        citations: None,
                    });
                }
                if let Some(tcs) = &msg.tool_calls {
                    for tc in tcs {
                        blocks.push(ContentBlock::ToolUse {
                            id: tc.id.clone(),
                            name: tc.name.clone(),
                            input: serde_json::from_str(&tc.arguments)
                                .unwrap_or_else(|_| serde_json::json!({})),
                        });
                    }
                }
                // Content blocks carry everything; do not also set tool_calls
                // (build_anthropic_messages would emit tool_use twice).
                ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    content_blocks: Some(blocks),
                    tool_calls: None,
                    tool_call_id: None,
                    reasoning_content: msg.reasoning_content.clone(),
                    name: None,
                }
            } else {
                let mut m = if let Some(tcs) = &msg.tool_calls {
                    let calls = tcs
                        .iter()
                        .map(|tc| anthropic_client_rs::ToolCall {
                            id: tc.id.clone(),
                            call_type: "function".into(),
                            name: tc.name.clone(),
                            input: serde_json::from_str(&tc.arguments)
                                .unwrap_or_else(|_| serde_json::json!({})),
                        })
                        .collect();
                    ChatMessage::assistant_with_tools(content, calls)
                } else {
                    ChatMessage::assistant(content)
                };
                if let Some(r) = &msg.reasoning_content {
                    m.reasoning_content = Some(r.clone());
                }
                m
            }
        }
        "tool" => ChatMessage::tool_result(msg.tool_call_id.clone().unwrap_or_default(), content),
        _ => {
            if let Some(images) = &msg.images {
                let mut blocks = vec![ContentBlock::Text {
                    text: content,
                    citations: None,
                }];
                for img in images {
                    match (&img.base64, &img.url) {
                        (Some(b64), _) => {
                            let media_type =
                                img.media_type.clone().unwrap_or_else(|| "image/png".into());
                            blocks.push(ContentBlock::Image {
                                source: ImageSource {
                                    source_type: "base64".into(),
                                    media_type,
                                    data: b64.clone(),
                                },
                            });
                        }
                        (None, Some(url)) => {
                            // Anthropic requires base64; URL-only images can't be
                            // fetched here, so surface a warning and skip.
                            tracing::warn!(
                                "Anthropic gateway needs base64 image data; skipping URL image {url}"
                            );
                        }
                        (None, None) => {}
                    }
                }
                ChatMessage::user_with_blocks(blocks)
            } else {
                ChatMessage::user(content)
            }
        }
    }
}

/// Convert a normalized tool schema into an Anthropic `Tool`.
pub fn to_anthropic_tool(schema: &ToolSchema) -> Tool {
    Tool::new(
        schema.name.clone(),
        schema.description.clone(),
        schema.parameters.clone(),
    )
}

/// Anthropic provider.
pub struct AnthropicProvider {
    inner: AnthropicAsyncClient,
}

impl AnthropicProvider {
    pub fn new(api_key: String, model: String, base_url: String) -> Self {
        let client = AnthropicAsyncClient::with_base_url(api_key, model, base_url);
        AnthropicProvider { inner: client }
    }

    fn tools_slice<'a>(&'a self, req: &'a LlmRequest) -> Option<Vec<Tool>> {
        if req.tools.is_empty() {
            None
        } else {
            Some(req.tools.iter().map(to_anthropic_tool).collect())
        }
    }

    fn max_tokens(&self, req: &LlmRequest) -> u64 {
        req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS)
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn chat(&self, req: &LlmRequest) -> Result<LlmResponse> {
        let messages: Vec<ChatMessage> = req.messages.iter().map(to_anthropic_message).collect();
        let tools = self.tools_slice(req);
        let system = req.system.as_deref();
        let max_tokens = self.max_tokens(req);

        for cfg in candidates_from_cache(&req.model, req.thinking) {
            let mut resp = match self
                .inner
                .messages_create(
                    &messages,
                    system,
                    tools.as_deref(),
                    max_tokens,
                    cfg.as_ref(),
                )
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    let err = Error::Llm(e.to_string());
                    if is_thinking_rejected(&err.to_string(), thinking_label(&cfg)) {
                        continue;
                    }
                    return Err(err);
                }
            };

            let text = std::mem::take(&mut resp.text);
            let reasoning = resp.reasoning_content.take();
            let finish_reason = resp.finish_reason.take();
            let tool_calls = resp
                .tool_calls
                .drain(..)
                .map(|tc| ToolCall {
                    id: tc.id,
                    name: tc.name,
                    arguments: tc.arguments,
                })
                .collect();
            let thinking_signature = signature_from_blocks(&resp.content_blocks);
            cache_mode(&req.model, &cfg);
            return Ok(LlmResponse {
                text,
                tool_calls,
                reasoning,
                finish_reason,
                thinking_signature,
            });
        }
        Err(Error::Llm("all thinking modes rejected".to_string()))
    }

    async fn chat_stream(&self, req: &LlmRequest, sink: &mut dyn LlmSink) -> Result<LlmResponse> {
        let messages: Vec<ChatMessage> = req.messages.iter().map(to_anthropic_message).collect();
        let tools = self.tools_slice(req);
        let system = req.system.as_deref();
        let max_tokens = self.max_tokens(req);

        for cfg in candidates_from_cache(&req.model, req.thinking) {
            match self
                .stream_once(
                    &messages,
                    system,
                    tools.as_deref(),
                    max_tokens,
                    cfg.as_ref(),
                    sink,
                )
                .await
            {
                Ok(resp) => {
                    cache_mode(&req.model, &cfg);
                    return Ok(resp);
                }
                Err(e) if is_thinking_rejected(&e.to_string(), thinking_label(&cfg)) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(Error::Llm("all thinking modes rejected".to_string()))
    }
}

impl AnthropicProvider {
    /// A single streaming request attempt with the given thinking config.
    async fn stream_once(
        &self,
        messages: &[ChatMessage],
        system: Option<&str>,
        tools: Option<&[Tool]>,
        max_tokens: u64,
        thinking: Option<&ThinkingConfig>,
        sink: &mut dyn LlmSink,
    ) -> Result<LlmResponse> {
        let mut stream = self
            .inner
            .messages_stream_events(messages, system, tools, max_tokens, thinking)
            .await
            .map_err(|e| Error::Llm(e.to_string()))?;

        let mut text = String::new();
        let mut reasoning = String::new();
        let mut thinking_signature: Option<String> = None;
        let mut finish_reason = None;
        let mut tool_started = false;
        // index -> (id, name, accumulated partial_json)
        let mut blocks: BTreeMap<i64, (String, String, String)> = BTreeMap::new();

        while let Some(ev) = stream.next().await {
            let ev = ev.map_err(|e| Error::Llm(e.to_string()))?;
            match ev.event_type.as_str() {
                "content_block_start" => {
                    if let (Some(i), Some(ContentBlock::ToolUse { id, name, .. })) =
                        (ev.index, ev.content_block)
                    {
                        if !tool_started {
                            tool_started = true;
                            sink.on_tool_call_start();
                        }
                        blocks.insert(i, (id, name, String::new()));
                    }
                }
                "content_block_delta" => {
                    if let (Some(i), Some(d)) = (ev.index, ev.delta) {
                        match d.delta_type.as_deref() {
                            Some("text_delta") => {
                                if let Some(t) = d.text {
                                    text.push_str(&t);
                                    sink.on_text(&t);
                                }
                            }
                            Some("thinking_delta") => {
                                if let Some(t) = d.thinking {
                                    reasoning.push_str(&t);
                                    sink.on_thinking(&t);
                                }
                            }
                            Some("signature_delta") => {
                                if let Some(sig) = d.signature {
                                    if !sig.is_empty() {
                                        thinking_signature = Some(sig);
                                    }
                                }
                            }
                            Some("input_json_delta") => {
                                if let Some(p) = d.partial_json {
                                    blocks
                                        .entry(i)
                                        .or_insert_with(|| {
                                            (String::new(), String::new(), String::new())
                                        })
                                        .2
                                        .push_str(&p);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                "message_delta" => {
                    if let Some(d) = &ev.delta {
                        if let Some(sr) = &d.stop_reason {
                            finish_reason = Some(sr.clone());
                        }
                    }
                }
                "error" => {
                    let msg = ev
                        .error
                        .map(|e| e.message)
                        .unwrap_or_else(|| "stream error".to_string());
                    return Err(Error::Llm(msg));
                }
                _ => {}
            }
        }

        let tool_calls = blocks
            .into_iter()
            .filter(|(_, (_, name, _))| !name.is_empty())
            .map(|(i, (id, name, arguments))| ToolCall {
                id: if id.is_empty() {
                    format!("call_{i}")
                } else {
                    id
                },
                name,
                arguments: if arguments.trim().is_empty() {
                    "{}".to_string()
                } else {
                    arguments
                },
            })
            .collect();

        Ok(LlmResponse {
            text,
            tool_calls,
            reasoning: if reasoning.is_empty() {
                None
            } else {
                Some(reasoning)
            },
            finish_reason,
            thinking_signature,
        })
    }
}

/// Extract the thinking signature from non-streaming content blocks.
fn signature_from_blocks(blocks: &[ContentBlock]) -> Option<String> {
    blocks.iter().find_map(|b| {
        if let ContentBlock::Thinking { signature, .. } = b {
            if signature.is_empty() {
                None
            } else {
                Some(signature.clone())
            }
        } else {
            None
        }
    })
}

/// Compute the candidate list, honoring the per-model cached mode (skip
/// already-rejected modes on subsequent calls).
fn candidates_from_cache(model: &str, pref: ThinkingPref) -> Vec<Option<ThinkingConfig>> {
    let candidates = thinking_candidates(pref);
    let cached = thinking_mode_cache()
        .lock()
        .map(|c| c.get(model).cloned())
        .unwrap_or_default();
    if let Some(cached_label) = cached {
        // Start from the cached mode if present in the candidate list.
        if let Some(pos) = candidates
            .iter()
            .position(|c| thinking_label(c) == cached_label)
        {
            return candidates[pos..].to_vec();
        }
    }
    candidates
}

fn cache_mode(model: &str, cfg: &Option<ThinkingConfig>) {
    if let Ok(mut c) = thinking_mode_cache().lock() {
        c.insert(model.to_string(), thinking_label(cfg).to_string());
    }
}
