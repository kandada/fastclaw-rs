// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! LLM message / tool-schema conversion.

use fastclaw::llm::provider::Message;
use fastclaw::ToolSchema;
use serde_json::json;

#[test]
fn message_from_value_roundtrip() {
    let v = json!({
        "role": "assistant",
        "content": "hi",
        "reasoning_content": "thinking...",
        "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "run_shell", "arguments": "{\"command\":\"ls\"}"}}]
    });
    let m = Message::from_value(&v);
    assert_eq!(m.role, "assistant");
    assert_eq!(m.reasoning_content.as_deref(), Some("thinking..."));
    assert_eq!(m.tool_calls.as_ref().unwrap()[0].name, "run_shell");
    assert_eq!(
        m.tool_calls.as_ref().unwrap()[0].arguments,
        "{\"command\":\"ls\"}"
    );

    let back = m.to_value();
    assert_eq!(back["role"], "assistant");
    assert_eq!(back["tool_calls"][0]["function"]["name"], "run_shell");
}

#[test]
fn message_tool_role() {
    let m = Message::tool("c1", "result");
    let v = m.to_value();
    assert_eq!(v["role"], "tool");
    assert_eq!(v["tool_call_id"], "c1");
}

#[cfg(feature = "openai")]
#[test]
fn openai_message_conversion() {
    let m = Message {
        role: "assistant".into(),
        content: "hi".into(),
        images: None,
        tool_calls: Some(vec![fastclaw::llm::provider::MessageToolCall {
            id: "c1".into(),
            name: "run_shell".into(),
            arguments: "{}".into(),
        }]),
        tool_call_id: None,
        reasoning_content: Some("think".into()),
        thinking_signature: None,
    };
    let cm = fastclaw::llm::openai::to_openai_message(&m);
    assert_eq!(cm.role, "assistant");
    assert!(cm.tool_calls.is_some());
    assert_eq!(cm.reasoning_content.as_deref(), Some("think"));
}

#[cfg(feature = "anthropic")]
#[test]
fn anthropic_message_conversion() {
    let m = Message::tool("c1", "result");
    let cm = fastclaw::llm::anthropic::to_anthropic_message(&m);
    assert_eq!(cm.role, "tool");
    assert_eq!(cm.tool_call_id.as_deref(), Some("c1"));
}

#[test]
fn tool_schema_shape() {
    let schema = ToolSchema {
        name: "run_shell".into(),
        description: "run a command".into(),
        parameters: json!({"type": "object", "properties": {}}),
    };
    #[cfg(feature = "openai")]
    {
        let t = fastclaw::llm::openai::to_openai_tool(&schema);
        assert_eq!(t.function.name, "run_shell");
    }
    #[cfg(feature = "anthropic")]
    {
        let t = fastclaw::llm::anthropic::to_anthropic_tool(&schema);
        assert_eq!(t.name, "run_shell");
    }
}

#[test]
fn image_part_constructors() {
    let by_url = fastclaw::ImagePart::from_url("https://example.com/a.png");
    assert!(by_url.is_valid());
    let by_b64 = fastclaw::ImagePart::from_base64("image/png", "AAAA");
    assert!(by_b64.is_valid());
    assert!(!fastclaw::ImagePart::default().is_valid());
}

#[test]
fn message_images_roundtrip() {
    let m = Message::user_with_images(
        "what is this?",
        vec![fastclaw::ImagePart::from_base64("image/png", "AAAA")],
    );
    let v = m.to_value();
    let images = v["images"].as_array().unwrap();
    assert_eq!(images.len(), 1);
    assert_eq!(images[0]["base64"], "AAAA");

    let back = Message::from_value(&v);
    assert_eq!(back.images.as_ref().unwrap().len(), 1);
    assert_eq!(
        back.images.as_ref().unwrap()[0].base64.as_deref(),
        Some("AAAA")
    );
}

#[cfg(feature = "openai")]
#[test]
fn openai_message_with_image_becomes_parts() {
    let m = Message::user_with_images(
        "look",
        vec![fastclaw::ImagePart::from_base64("image/png", "QUJD")],
    );
    let cm = fastclaw::llm::openai::to_openai_message(&m);
    let parts = cm.content_parts.expect("content parts");
    assert_eq!(parts.len(), 2); // text + image
    match &parts[1] {
        openai_client_rs::ContentPart::ImageUrl { image_url } => {
            assert!(image_url.url.starts_with("data:image/png;base64,QUJD"));
        }
        _ => panic!("expected image part"),
    }
}

#[cfg(feature = "openai")]
#[test]
fn openai_message_with_url_image() {
    let m = Message::user_with_images(
        "look",
        vec![fastclaw::ImagePart::from_url("https://example.com/a.png")],
    );
    let cm = fastclaw::llm::openai::to_openai_message(&m);
    let parts = cm.content_parts.unwrap();
    match &parts[1] {
        openai_client_rs::ContentPart::ImageUrl { image_url } => {
            assert_eq!(image_url.url, "https://example.com/a.png");
        }
        _ => panic!("expected image part"),
    }
}

#[cfg(feature = "anthropic")]
#[test]
fn anthropic_message_with_image_becomes_blocks() {
    let m = Message::user_with_images(
        "look",
        vec![fastclaw::ImagePart::from_base64("image/png", "QUJD")],
    );
    let cm = fastclaw::llm::anthropic::to_anthropic_message(&m);
    let blocks = cm.content_blocks.unwrap();
    assert_eq!(blocks.len(), 2); // text + image
    match &blocks[1] {
        anthropic_client_rs::ContentBlock::Image { source } => {
            assert_eq!(source.source_type, "base64");
            assert_eq!(source.media_type, "image/png");
            assert_eq!(source.data, "QUJD");
        }
        _ => panic!("expected image block"),
    }
}

// ── thinking / reasoning extraction ─────────────────────────────────────────

#[cfg(feature = "openai")]
#[test]
fn openai_thinking_field_recognized() {
    use openai_client_rs::thinking::{extract_thinking, ThinkTagParser};
    // MiniMax-style `thinking` field (not `reasoning_content`).
    let d = json!({ "thinking": "let me plan", "content": "answer" });
    let mut p = ThinkTagParser::default();
    let (t, c) = extract_thinking(&d, &mut p);
    assert_eq!(t, "let me plan");
    assert_eq!(c, "answer");
}

#[cfg(feature = "openai")]
#[test]
fn openai_reasoning_field_recognized() {
    use openai_client_rs::thinking::{extract_thinking, ThinkTagParser};
    let d = json!({ "reasoning": "R", "content": "C" });
    let mut p = ThinkTagParser::default();
    let (t, c) = extract_thinking(&d, &mut p);
    assert_eq!(t, "R");
    assert_eq!(c, "C");
}

#[cfg(feature = "openai")]
#[test]
fn openai_think_tags_stripped_across_chunks() {
    use openai_client_rs::thinking::{extract_thinking, ThinkTagParser};
    let mut p = ThinkTagParser::default();
    let (t1, c1) = extract_thinking(&json!({ "content": "<thi" }), &mut p);
    let (t2, c2) = extract_thinking(&json!({ "content": "nk>deep</think>visible" }), &mut p);
    let (t3, c3) = p.flush();
    assert_eq!(format!("{t1}{t2}{t3}"), "deep");
    assert_eq!(format!("{c1}{c2}{c3}"), "visible");
}

#[cfg(feature = "openai")]
#[test]
fn openai_field_priority_over_tags() {
    use openai_client_rs::thinking::{extract_thinking, ThinkTagParser};
    // Field present + inline tags in content: field wins, tags stripped.
    let d = json!({ "reasoning_content": "FIELD", "content": "<think>TAG</think>answer" });
    let mut p = ThinkTagParser::default();
    let (t, c) = extract_thinking(&d, &mut p);
    assert_eq!(t, "FIELD");
    assert_eq!(c, "answer");
}

// ── thinking_signature on messages ──────────────────────────────────────────

#[test]
fn message_thinking_signature_roundtrip() {
    let v = json!({
        "role": "assistant",
        "content": "hi",
        "reasoning_content": "think",
        "thinking_signature": "sig-123",
    });
    let m = Message::from_value(&v);
    assert_eq!(m.thinking_signature.as_deref(), Some("sig-123"));
    let back = m.to_value();
    assert_eq!(back["thinking_signature"], "sig-123");
}

#[cfg(feature = "anthropic")]
#[test]
fn anthropic_assistant_rebuilds_thinking_block() {
    use anthropic_client_rs::ContentBlock;
    use fastclaw::llm::anthropic::to_anthropic_message;
    use fastclaw::llm::provider::Message;

    let m = Message {
        role: "assistant".into(),
        content: "visible".into(),
        images: None,
        tool_calls: None,
        tool_call_id: None,
        reasoning_content: Some("deep think".into()),
        thinking_signature: Some("sig-abc".into()),
    };
    let cm = to_anthropic_message(&m);
    let blocks = cm.content_blocks.expect("content blocks");
    // [thinking, text]
    assert_eq!(blocks.len(), 2);
    match &blocks[0] {
        ContentBlock::Thinking {
            thinking,
            signature,
        } => {
            assert_eq!(thinking, "deep think");
            assert_eq!(signature, "sig-abc");
        }
        _ => panic!("expected thinking block"),
    }
    match &blocks[1] {
        ContentBlock::Text { text, .. } => assert_eq!(text, "visible"),
        _ => panic!("expected text block"),
    }
}

// ── anthropic thinking mode selection / rejection detection ────────────────

#[cfg(feature = "anthropic")]
#[test]
fn anthropic_thinking_candidates_default_adaptive() {
    use fastclaw::llm::anthropic::thinking_candidates;
    use fastclaw::ThinkingPref;

    let c = thinking_candidates(ThinkingPref::Adaptive);
    // adaptive → enabled → none
    assert_eq!(c.len(), 3);
    assert!(matches!(
        c[0],
        Some(anthropic_client_rs::ThinkingConfig::Adaptive { .. })
    ));
    assert!(matches!(
        c[1],
        Some(anthropic_client_rs::ThinkingConfig::Enabled { .. })
    ));
    assert!(c[2].is_none());

    let d = thinking_candidates(ThinkingPref::Disabled);
    assert_eq!(d.len(), 1);
    assert!(d[0].is_none());

    let e = thinking_candidates(ThinkingPref::Enabled(8192));
    assert_eq!(e.len(), 2);
    assert!(matches!(
        e[0],
        Some(anthropic_client_rs::ThinkingConfig::Enabled { .. })
    ));
    assert!(e[1].is_none());
}

#[cfg(feature = "anthropic")]
#[test]
fn anthropic_thinking_rejection_detection() {
    use fastclaw::llm::anthropic::is_thinking_rejected;

    // Official Anthropic 4.7+ rejects `enabled` (message carries the literal
    // `"thinking.type.enabled"` string).
    assert!(is_thinking_rejected(
        "api error: HTTP 400: {\"error\":{\"message\":\"thinking.type.enabled\" is not supported}}",
        "enabled"
    ));
    // Official Anthropic 4.5- rejects `adaptive`.
    assert!(is_thinking_rejected(
        "api error: HTTP 400: {\"error\":{\"message\":\"thinking.type.adaptive\" is not supported}}",
        "adaptive"
    ));
    // Third-party proxy rejects the whole field.
    assert!(is_thinking_rejected("unknown field: thinking", "adaptive"));
    // Generic fallback.
    assert!(is_thinking_rejected(
        "thinking is not supported by this model",
        "adaptive"
    ));
    // Authentication / quota / model-not-found errors are NOT treated as
    // thinking rejection (so they propagate instead of downgrading).
    assert!(!is_thinking_rejected(
        "api error: HTTP 401: invalid api key",
        "adaptive"
    ));
    assert!(!is_thinking_rejected(
        "api error: HTTP 429: rate limit exceeded",
        "adaptive"
    ));
    assert!(!is_thinking_rejected(
        "api error: HTTP 404: model not found",
        "adaptive"
    ));
    // A rejection naming a DIFFERENT mode means the current mode is fine.
    assert!(!is_thinking_rejected(
        "{\"error\":{\"message\":\"thinking.type.adaptive\" is not supported}}",
        "enabled"
    ));
}
