// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Message persistence + context management.

use fastclaw::session::{
    calculate_tokens, count_messages_tokens, fix_invalid_tool_calls, load_messages_from_jsonl,
    save_messages_to_jsonl, unload_early_messages,
};
use serde_json::json;

#[test]
fn token_estimation() {
    assert_eq!(calculate_tokens("12345678"), 2);
    assert_eq!(calculate_tokens(""), 0);
    assert_eq!(calculate_tokens("abcdefghijkl"), 3);
}

#[test]
fn count_tokens_across_messages() {
    let msgs = vec![
        json!({"role": "user", "content": "12345678"}), // 2
        json!({"role": "assistant", "content": "abcd", "reasoning_content": "12345678"}), // 1 + 2
    ];
    assert_eq!(count_messages_tokens(&msgs), 5);
}

#[test]
fn fix_invalid_removes_unanswered_tool_calls() {
    let msgs = vec![
        json!({"role": "user", "content": "hi"}),
        json!({"role": "assistant", "content": "", "tool_calls": [{"id": "c1", "function": {"name": "run_shell", "arguments": "{}"}}]}),
        // missing the tool response for c1
        json!({"role": "user", "content": "next"}),
    ];
    let fixed = fix_invalid_tool_calls(&msgs);
    assert!(fixed[1].get("tool_calls").is_none());
}

#[test]
fn fix_invalid_keeps_answered_tool_calls() {
    let msgs = vec![
        json!({"role": "assistant", "content": "", "tool_calls": [{"id": "c1", "function": {"name": "run_shell", "arguments": "{}"}}]}),
        json!({"role": "tool", "tool_call_id": "c1", "content": "ok"}),
    ];
    let fixed = fix_invalid_tool_calls(&msgs);
    assert!(fixed[0].get("tool_calls").is_some());
}

#[test]
fn unload_keeps_recent_half() {
    let mut msgs = Vec::new();
    for i in 0..20 {
        msgs.push(json!({"role": "user", "content": format!("message number {i} with some padding text")}));
    }
    let (kept, unloaded) = unload_early_messages(&msgs, 10);
    assert!(!kept.is_empty());
    assert!(!unloaded.is_empty());
    assert_eq!(kept.len() + unloaded.len(), 20);
}

#[test]
fn unload_noop_under_threshold() {
    let msgs = vec![json!({"role": "user", "content": "short"})];
    let (kept, unloaded) = unload_early_messages(&msgs, 1_000_000);
    assert_eq!(kept.len(), 1);
    assert!(unloaded.is_empty());
}

#[test]
fn save_load_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let msgs = vec![
        json!({"role": "user", "content": "hello"}),
        json!({"role": "assistant", "content": "hi there", "tool_calls": [{"id": "c1", "function": {"name": "x", "arguments": "{}"}}]}),
    ];
    save_messages_to_jsonl(dir.path(), "s1", &msgs);
    let loaded = load_messages_from_jsonl(dir.path(), "s1");
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0]["content"], "hello");
    // Timestamp is backfilled on save.
    assert!(loaded[0].get("timestamp").is_some());
}

#[test]
fn load_missing_returns_empty() {
    let dir = tempfile::tempdir().unwrap();
    assert!(load_messages_from_jsonl(dir.path(), "nonexistent").is_empty());
}

#[test]
fn first_user_text_extracted() {
    let msgs = vec![
        json!({"role": "assistant", "content": "skip"}),
        json!({"role": "user", "content": "x".repeat(120)}),
    ];
    let first = fastclaw::session::messages::first_user_text(&msgs);
    assert!(first.is_some());
    assert_eq!(first.unwrap().len(), 50); // truncated to 50 chars
}

#[test]
fn unload_never_cuts_tool_roundtrip() {
    // Build a history with an assistant(tool_calls) → tool pair in the middle.
    let mut msgs = Vec::new();
    for i in 0..10 {
        msgs.push(json!({"role": "user", "content": format!("padding message {i} with extra text to add tokens")}));
    }
    msgs.push(json!({"role": "assistant", "content": "", "tool_calls": [{"id": "c1", "function": {"name": "run_shell", "arguments": "{}"}}]}));
    msgs.push(json!({"role": "tool", "tool_call_id": "c1", "content": "result"}));
    for i in 0..10 {
        msgs.push(json!({"role": "user", "content": format!("tail message {i} with extra text to add tokens")}));
    }

    let (kept, _unloaded) = unload_early_messages(&msgs, 5);
    // No `tool` message may appear without its preceding assistant(tool_calls).
    for (i, m) in kept.iter().enumerate() {
        if m["role"] == "tool" {
            assert!(
                i > 0 && kept[i - 1].get("tool_calls").is_some(),
                "tool message must follow an assistant with tool_calls"
            );
        }
    }
}

#[test]
fn fix_invalid_skips_leading_tool_messages() {
    // Leading orphaned tool messages are dropped (matching Python).
    let msgs = vec![
        json!({"role": "tool", "tool_call_id": "orphan", "content": "x"}),
        json!({"role": "user", "content": "hi"}),
    ];
    let fixed = fix_invalid_tool_calls(&msgs);
    assert_eq!(fixed.len(), 1);
    assert_eq!(fixed[0]["content"], "hi");
}

#[test]
fn count_tokens_includes_tool_calls() {
    let msgs = vec![json!({
        "role": "assistant",
        "content": "",
        "tool_calls": [{"id": "c1", "function": {"name": "run_shell", "arguments": "{\"command\":\"ls\"}"}}]
    })];
    assert!(count_messages_tokens(&msgs) > 0);
}
