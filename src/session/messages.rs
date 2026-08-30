// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Message persistence (`messages.jsonl`) and context-management helpers.
//!
//! Messages are stored as JSON objects (role / content / tool_calls /
//! tool_call_id / reasoning_content / timestamp) — identical to the Python
//! fastclaw format for drop-in compatibility.

use std::path::Path;

use chrono::Utc;
use serde_json::{json, Value};

/// Approximate token count: `len(text) / 4` (matches Python fastclaw).
pub fn calculate_tokens(text: &str) -> usize {
    text.len() / 4
}

pub fn count_messages_tokens(messages: &[Value]) -> usize {
    let mut total = 0;
    for msg in messages {
        total += calculate_tokens(msg.get("content").and_then(|v| v.as_str()).unwrap_or(""));
        total += calculate_tokens(
            msg.get("reasoning_content")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
        );
        if let Some(tc) = msg.get("tool_calls") {
            total += calculate_tokens(&tc.to_string());
        }
    }
    total
}

/// Fix malformed history: if an assistant message has tool_calls whose ids are
/// not all answered by the following tool messages, drop the tool_calls field
/// (mirrors Python `fix_invalid_tool_calls`).
pub fn fix_invalid_tool_calls(messages: &[Value]) -> Vec<Value> {
    let mut fixed = Vec::new();
    let mut i = 0;
    while i < messages.len() && messages[i].get("role").and_then(|v| v.as_str()) == Some("tool") {
        i += 1;
    }
    while i < messages.len() {
        let msg = &messages[i];
        if msg.get("role").and_then(|v| v.as_str()) == Some("assistant")
            && msg
                .get("tool_calls")
                .and_then(|v| v.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false)
        {
            let tool_call_ids: std::collections::HashSet<&str> = msg["tool_calls"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|tc| tc.get("id").and_then(|v| v.as_str()))
                .collect();
            if tool_call_ids.is_empty() {
                fixed.push(msg.clone());
                i += 1;
                continue;
            }
            let mut j = i + 1;
            let mut responded = std::collections::HashSet::new();
            while j < messages.len()
                && messages[j].get("role").and_then(|v| v.as_str()) == Some("tool")
            {
                if let Some(tid) = messages[j].get("tool_call_id").and_then(|v| v.as_str()) {
                    responded.insert(tid);
                }
                j += 1;
            }
            if tool_call_ids.iter().all(|id| responded.contains(id)) {
                fixed.push(msg.clone());
            } else {
                let mut copy = msg.clone();
                if let Some(obj) = copy.as_object_mut() {
                    obj.remove("tool_calls");
                }
                fixed.push(copy);
                while i + 1 < messages.len()
                    && messages[i + 1].get("role").and_then(|v| v.as_str()) == Some("tool")
                {
                    i += 1;
                }
            }
            i += 1;
        } else {
            fixed.push(msg.clone());
            i += 1;
        }
    }
    fixed
}

/// Unload early messages when over the threshold, keeping a safe split point
/// that never cuts in the middle of a tool round-trip.
pub fn unload_early_messages(messages: &[Value], threshold: usize) -> (Vec<Value>, Vec<Value>) {
    if count_messages_tokens(messages) < threshold {
        return (messages.to_vec(), vec![]);
    }
    let mut boundary = messages.len() / 2;
    // Move forward past trailing tool responses.
    while boundary < messages.len()
        && messages[boundary].get("role").and_then(|v| v.as_str()) == Some("tool")
    {
        boundary += 1;
    }
    // Move backward if the preceding assistant message carries tool_calls.
    while boundary > 0
        && messages[boundary - 1].get("role").and_then(|v| v.as_str()) == Some("assistant")
        && messages[boundary - 1]
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false)
    {
        boundary -= 1;
        while boundary > 0
            && messages[boundary - 1].get("role").and_then(|v| v.as_str()) == Some("tool")
        {
            boundary -= 1;
        }
    }
    (messages[boundary..].to_vec(), messages[..boundary].to_vec())
}

/// Persist messages to `sessions/<id>/messages.jsonl` (one JSON per line).
pub fn save_messages_to_jsonl(sessions_dir: &Path, session_id: &str, messages: &[Value]) {
    let dir = sessions_dir.join(session_id);
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join("messages.jsonl");
    let mut out = String::new();
    for msg in messages {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if !matches!(role, "user" | "assistant" | "system" | "tool") {
            continue;
        }
        let mut msg = msg.clone();
        if msg.get("timestamp").is_none() {
            if let Some(obj) = msg.as_object_mut() {
                obj.insert("timestamp".to_string(), json!(Utc::now().to_rfc3339()));
            }
        }
        out.push_str(&msg.to_string());
        out.push('\n');
    }
    let _ = std::fs::write(file, out);
}

/// Load messages from `sessions/<id>/messages.jsonl`, returning `[]` on any error.
pub fn load_messages_from_jsonl(sessions_dir: &Path, session_id: &str) -> Vec<Value> {
    let file = sessions_dir.join(session_id).join("messages.jsonl");
    let content = match std::fs::read_to_string(file) {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect()
}

/// The first user-message snippet used to auto-name a session (first 50 chars).
pub fn first_user_text(messages: &[Value]) -> Option<String> {
    messages
        .iter()
        .find(|m| m.get("role").and_then(|v| v.as_str()) == Some("user"))
        .and_then(|m| m.get("content").and_then(|v| v.as_str()))
        .map(|c| c.chars().take(50).collect())
}
