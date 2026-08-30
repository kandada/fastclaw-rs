// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! OpenAI-compatible gateway adapter (DeepSeek / Kimi / MiniMax / Ollama …).

use std::collections::BTreeMap;

use async_trait::async_trait;
use futures::StreamExt;
use openai_client_rs::{ChatMessage, ContentPart, ImageUrl, OpenAiAsyncClient, Tool};
use serde_json::json;

use super::provider::{
    LlmProvider, LlmRequest, LlmResponse, LlmSink, Message, ToolCall, ToolSchema,
};
use crate::error::{Error, Result};

/// Convert a normalized message into an OpenAI `ChatMessage`.
pub fn to_openai_message(msg: &Message) -> ChatMessage {
    let content = msg.content.clone();
    match msg.role.as_str() {
        "system" => ChatMessage::system(content),
        "assistant" => {
            let mut m = if let Some(tcs) = &msg.tool_calls {
                let calls = tcs
                    .iter()
                    .map(|tc| openai_client_rs::ToolCall {
                        id: tc.id.clone(),
                        call_type: "function".into(),
                        function: openai_client_rs::FunctionCall {
                            name: tc.name.clone(),
                            arguments: tc.arguments.clone(),
                        },
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
        "tool" => ChatMessage::tool_result(msg.tool_call_id.clone().unwrap_or_default(), content),
        _ => {
            if let Some(images) = &msg.images {
                let mut parts = vec![ContentPart::Text { text: content }];
                for img in images {
                    let url = match (&img.base64, &img.url) {
                        (Some(b64), _) => {
                            let mt = img.media_type.as_deref().unwrap_or("image/png");
                            format!("data:{mt};base64,{b64}")
                        }
                        (None, Some(u)) => u.clone(),
                        (None, None) => continue,
                    };
                    parts.push(ContentPart::ImageUrl {
                        image_url: ImageUrl {
                            url,
                            // Omit `detail` (OpenAI-specific); MiniMax et al reject it.
                            detail: None,
                        },
                    });
                }
                ChatMessage::user_with_parts(parts)
            } else {
                ChatMessage::user(content)
            }
        }
    }
}

/// Convert a normalized tool schema into an OpenAI `Tool`.
pub fn to_openai_tool(schema: &ToolSchema) -> Tool {
    Tool::function(
        schema.name.clone(),
        schema.description.clone(),
        schema.parameters.clone(),
    )
}

/// OpenAI-compatible provider.
pub struct OpenAiProvider {
    inner: OpenAiAsyncClient,
}

impl OpenAiProvider {
    pub fn new(api_key: String, model: String, base_url: String) -> Self {
        let client = OpenAiAsyncClient::with_base_url(api_key, model, base_url);
        OpenAiProvider { inner: client }
    }

    fn tools_slice<'a>(&'a self, req: &'a LlmRequest) -> Option<Vec<Tool>> {
        if req.tools.is_empty() {
            None
        } else {
            Some(req.tools.iter().map(to_openai_tool).collect())
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn chat(&self, req: &LlmRequest) -> Result<LlmResponse> {
        let messages: Vec<ChatMessage> = req.messages.iter().map(to_openai_message).collect();
        let tools = self.tools_slice(req);
        let mut resp = self
            .inner
            .chat_create(&messages, tools.as_deref())
            .await
            .map_err(|e| Error::Llm(e.to_string()))?;

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
        Ok(LlmResponse {
            text,
            tool_calls,
            reasoning,
            finish_reason,
            thinking_signature: None,
        })
    }

    async fn chat_stream(&self, req: &LlmRequest, sink: &mut dyn LlmSink) -> Result<LlmResponse> {
        let messages: Vec<ChatMessage> = req.messages.iter().map(to_openai_message).collect();
        let tools = self.tools_slice(req);

        // Use chunk-level streaming so reasoning vs. content stay distinct
        // (the high-level `chat_stream` merges both into one `on_delta`).
        let mut chunks = self
            .inner
            .chat_stream_chunks(&messages, tools.as_deref())
            .await
            .map_err(|e| Error::Llm(e.to_string()))?;

        let mut text = String::new();
        let mut reasoning = String::new();
        let mut finish_reason = None;
        let mut tool_started = false;
        // index -> (id, name, accumulated arguments)
        let mut acc: BTreeMap<usize, (String, String, String)> = BTreeMap::new();
        // Unified thinking extraction: `reasoning_content` / `reasoning` /
        // `thinking` fields + `<think>...</think>` tags (stateful across chunks).
        let mut tag_parser = openai_client_rs::thinking::ThinkTagParser::default();

        while let Some(chunk) = chunks.next().await {
            let chunk = chunk.map_err(|e| Error::Llm(e.to_string()))?;
            let Some(choice) = chunk.choices.into_iter().next() else {
                continue;
            };
            if let Some(fr) = choice.finish_reason {
                finish_reason = Some(fr);
            }
            let d = choice.delta;
            // Feed the whole delta through the unified extractor.
            let delta_value = serde_json::to_value(&d).unwrap_or_else(|_| json!({}));
            let (think_chunk, content_chunk) =
                openai_client_rs::thinking::extract_thinking(&delta_value, &mut tag_parser);
            if !think_chunk.is_empty() {
                reasoning.push_str(&think_chunk);
                sink.on_thinking(&think_chunk);
            }
            if !content_chunk.is_empty() {
                text.push_str(&content_chunk);
                sink.on_text(&content_chunk);
            }
            if let Some(tcs) = d.tool_calls {
                if !tool_started {
                    tool_started = true;
                    sink.on_tool_call_start();
                }
                for tc in tcs {
                    let i = tc.index as usize;
                    let e = acc
                        .entry(i)
                        .or_insert_with(|| (String::new(), String::new(), String::new()));
                    if let Some(id) = tc.id {
                        if !id.is_empty() {
                            e.0 = id;
                        }
                    }
                    if let Some(f) = tc.function {
                        if let Some(n) = f.name {
                            if !n.is_empty() {
                                e.1.push_str(&n);
                            }
                        }
                        if let Some(a) = f.arguments {
                            e.2.push_str(&a);
                        }
                    }
                }
            }
        }

        // Flush any residual `<think>` tag buffer from cross-chunk splits.
        let (tail_think, tail_content) = tag_parser.flush();
        if !tail_think.is_empty() {
            reasoning.push_str(&tail_think);
            sink.on_thinking(&tail_think);
        }
        if !tail_content.is_empty() {
            text.push_str(&tail_content);
            sink.on_text(&tail_content);
        }

        let tool_calls = acc
            .into_iter()
            .filter(|(_, (_, name, _))| !name.is_empty())
            .map(|(i, (id, name, arguments))| ToolCall {
                id: if id.is_empty() {
                    format!("call_{i}")
                } else {
                    id
                },
                name,
                arguments,
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
            thinking_signature: None,
        })
    }
}
