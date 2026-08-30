// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! LLM gateway abstraction: a `LlmProvider` trait with OpenAI-compatible and
//! Anthropic adapters, sharing a normalized message / tool / response model.

pub mod provider;

#[cfg(feature = "openai")]
pub mod openai;

#[cfg(feature = "anthropic")]
pub mod anthropic;

pub use provider::{
    ImagePart, LlmGateway, LlmProvider, LlmRequest, LlmResponse, LlmSink, Message, ThinkingPref,
    ToolCall, ToolSchema,
};

/// Build an `Arc<dyn LlmProvider>` for the given gateway + config.
///
/// Returns `None` when the corresponding feature is disabled or no API key is
/// available (hosts/tests can then skip real LLM calls).
pub fn build_provider(
    gateway: LlmGateway,
    model: String,
    api_key: String,
    base_url: String,
) -> Option<std::sync::Arc<dyn LlmProvider>> {
    if api_key.is_empty() {
        return None;
    }
    match gateway {
        LlmGateway::OpenAi => {
            #[cfg(feature = "openai")]
            {
                Some(std::sync::Arc::new(openai::OpenAiProvider::new(
                    api_key, model, base_url,
                )))
            }
            #[cfg(not(feature = "openai"))]
            {
                let _ = (model, base_url);
                None
            }
        }
        LlmGateway::Anthropic => {
            #[cfg(feature = "anthropic")]
            {
                Some(std::sync::Arc::new(anthropic::AnthropicProvider::new(
                    api_key, model, base_url,
                )))
            }
            #[cfg(not(feature = "anthropic"))]
            {
                let _ = (model, base_url);
                None
            }
        }
    }
}
