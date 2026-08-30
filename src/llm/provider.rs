// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! The `LlmProvider` abstraction and normalized types.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::Result;

/// Which gateway an agent uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LlmGateway {
    #[default]
    OpenAi,
    Anthropic,
}

impl std::fmt::Display for LlmGateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmGateway::OpenAi => write!(f, "openai"),
            LlmGateway::Anthropic => write!(f, "anthropic"),
        }
    }
}

/// An image attached to a user message.
///
/// Either `url` (OpenAI gateway accepts remote URLs) or `base64` (+ `media_type`,
/// required by the Anthropic gateway) must be present. For OpenAI, a `base64`
/// image is sent as a `data:` URL automatically.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ImagePart {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

impl ImagePart {
    pub fn from_url(url: impl Into<String>) -> Self {
        ImagePart {
            url: Some(url.into()),
            base64: None,
            media_type: None,
        }
    }

    pub fn from_base64(media_type: impl Into<String>, data: impl Into<String>) -> Self {
        ImagePart {
            url: None,
            base64: Some(data.into()),
            media_type: Some(media_type.into()),
        }
    }

    /// Whether this image has enough information to be sent.
    pub fn is_valid(&self) -> bool {
        self.url.is_some() || self.base64.is_some()
    }
}

/// A single conversation message (normalized; bridges to either gateway).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<ImagePart>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<MessageToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_signature: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Message {
            role: "system".into(),
            content: content.into(),
            images: None,
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
            thinking_signature: None,
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Message {
            role: "user".into(),
            content: content.into(),
            images: None,
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
            thinking_signature: None,
        }
    }
    pub fn user_with_images(content: impl Into<String>, images: Vec<ImagePart>) -> Self {
        Message {
            role: "user".into(),
            content: content.into(),
            images: Some(images),
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
            thinking_signature: None,
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Message {
            role: "assistant".into(),
            content: content.into(),
            images: None,
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
            thinking_signature: None,
        }
    }
    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Message {
            role: "tool".into(),
            content: content.into(),
            images: None,
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
            reasoning_content: None,
            thinking_signature: None,
        }
    }

    /// Convert a JSONL message object (Python-compatible) into a `Message`.
    pub fn from_value(v: &Value) -> Message {
        let role = v.get("role").and_then(|v| v.as_str()).unwrap_or("user");
        let content = v.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let tool_calls = v.get("tool_calls").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|tc| {
                    let id = tc
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let f = tc.get("function")?;
                    Some(MessageToolCall {
                        id,
                        name: f
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        arguments: f
                            .get("arguments")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    })
                })
                .collect::<Vec<_>>()
        });
        let tool_call_id = v
            .get("tool_call_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let reasoning_content = v
            .get("reasoning_content")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let thinking_signature = v
            .get("thinking_signature")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let images = v
            .get("images")
            .and_then(|v| serde_json::from_value::<Vec<ImagePart>>(v.clone()).ok())
            .filter(|imgs| !imgs.is_empty());
        Message {
            role: role.to_string(),
            content: content.to_string(),
            images,
            tool_calls: if tool_calls.as_ref().map(|t| t.is_empty()).unwrap_or(true) {
                None
            } else {
                tool_calls
            },
            tool_call_id,
            reasoning_content,
            thinking_signature,
        }
    }

    /// Serialize back to a Python-compatible JSONL object.
    pub fn to_value(&self) -> Value {
        let mut obj = serde_json::Map::new();
        obj.insert("role".into(), Value::String(self.role.clone()));
        obj.insert("content".into(), Value::String(self.content.clone()));
        if let Some(images) = &self.images {
            if let Ok(arr) = serde_json::to_value(images) {
                obj.insert("images".into(), arr);
            }
        }
        if let Some(tcs) = &self.tool_calls {
            let arr: Vec<Value> = tcs
                .iter()
                .map(|tc| {
                    json!({
                        "id": tc.id,
                        "type": "function",
                        "function": { "name": tc.name, "arguments": tc.arguments },
                    })
                })
                .collect();
            obj.insert("tool_calls".into(), Value::Array(arr));
        }
        if let Some(id) = &self.tool_call_id {
            obj.insert("tool_call_id".into(), Value::String(id.clone()));
        }
        if let Some(r) = &self.reasoning_content {
            obj.insert("reasoning_content".into(), Value::String(r.clone()));
        }
        if let Some(sig) = &self.thinking_signature {
            obj.insert("thinking_signature".into(), Value::String(sig.clone()));
        }
        Value::Object(obj)
    }
}

/// A tool call within an assistant message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// A normalized tool definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// A normalized tool call returned by the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// A normalized completion response.
#[derive(Debug, Clone, Default)]
pub struct LlmResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub reasoning: Option<String>,
    pub finish_reason: Option<String>,
    /// Anthropic extended-thinking signature, needed to pass thinking blocks
    /// back in multi-turn conversations (matches the `thinking_signature`
    /// stored on the assistant message, as in Python fastclaw).
    pub thinking_signature: Option<String>,
}

/// Anthropic extended-thinking preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThinkingPref {
    /// Request `thinking: {type: adaptive}` (Claude 4.6+ / MiniMax, default).
    #[default]
    Adaptive,
    /// Request `thinking: {type: enabled, budget_tokens}` (Claude 4.5-).
    Enabled(u64),
    /// Do not request thinking.
    Disabled,
}

/// The request given to a provider.
#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSchema>,
    pub model: String,
    pub max_tokens: Option<u64>,
    /// Anthropic extended-thinking preference (ignored by the OpenAI gateway).
    pub thinking: ThinkingPref,
}

/// Streaming sink: the agent implements this to receive real-time deltas.
pub trait LlmSink: Send {
    /// A reasoning delta (thinking tokens).
    fn on_thinking(&mut self, delta: &str);
    /// A content delta (normal text).
    fn on_text(&mut self, delta: &str);
    /// Called once when the first tool-call fragment is observed, so callers can
    /// suppress further text deltas (mirrors Python's `has_tool_calls` flag).
    fn on_tool_call_start(&mut self) {}
}

/// A no-op sink for callers that only want the final response.
pub struct NullSink;

impl LlmSink for NullSink {
    fn on_thinking(&mut self, _delta: &str) {}
    fn on_text(&mut self, _delta: &str) {}
}

/// A pluggable LLM backend.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat(&self, req: &LlmRequest) -> Result<LlmResponse>;
    async fn chat_stream(&self, req: &LlmRequest, sink: &mut dyn LlmSink) -> Result<LlmResponse>;
}
