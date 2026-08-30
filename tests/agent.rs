// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! ReAct loop end-to-end (mock LLM).

use fastclaw::{FastClaw, FastClawConfig};

mod common;
use common::{MockProvider, MockReply};

fn build_claw(
    root: &std::path::Path,
    script: Vec<MockReply>,
) -> (FastClaw, std::sync::Arc<MockProvider>) {
    let claw = FastClaw::builder(FastClawConfig {
        workspace_root: Some(root.to_path_buf()),
        ..FastClawConfig::default()
    })
    .build();
    let mock = MockProvider::new(script);
    claw.set_mock_provider(mock.clone());
    (claw, mock)
}

#[tokio::test]
async fn react_tool_then_text() {
    let dir = tempfile::tempdir().unwrap();
    let (claw, mock) = build_claw(
        dir.path(),
        vec![
            MockReply::Tool(vec![(
                "run_shell".into(),
                "{\"command\":\"echo hi\"}".into(),
            )]),
            MockReply::Text("All done!".into()),
        ],
    );
    claw.start().await.unwrap();

    let sid = claw.new_session(None);
    let reply = claw.run_chat(&sid, "do something", None, None).await;
    assert_eq!(reply, "All done!");

    // The conversation contains a tool round-trip.
    let msgs = claw.get_messages(&sid);
    let roles: Vec<&str> = msgs.iter().filter_map(|m| m["role"].as_str()).collect();
    assert_eq!(roles, vec!["user", "assistant", "tool", "assistant"]);
    assert!(msgs[1].get("tool_calls").is_some());
    // Two LLM calls: initial + after tool result.
    assert_eq!(mock.call_count(), 2);

    claw.stop().await.unwrap();
}

#[tokio::test]
async fn react_text_only() {
    let dir = tempfile::tempdir().unwrap();
    let (claw, mock) = build_claw(dir.path(), vec![MockReply::Text("Hello!".into())]);
    claw.start().await.unwrap();

    let sid = claw.new_session(None);
    let reply = claw.run_chat(&sid, "hi", None, None).await;
    assert_eq!(reply, "Hello!");
    assert_eq!(mock.call_count(), 1);

    let msgs = claw.get_messages(&sid);
    assert_eq!(msgs.len(), 2); // user + assistant
    claw.stop().await.unwrap();
}

#[tokio::test]
async fn agent_rerun_does_not_duplicate_user_message() {
    let dir = tempfile::tempdir().unwrap();
    // Two LLM turns: first a tool call, then text. The agent node runs twice on
    // the same event; the user message must be appended only once.
    let (claw, _mock) = build_claw(
        dir.path(),
        vec![
            MockReply::Tool(vec![(
                "run_shell".into(),
                "{\"command\":\"echo hi\"}".into(),
            )]),
            MockReply::Text("done".into()),
        ],
    );
    claw.start().await.unwrap();
    let sid = claw.new_session(None);
    let _ = claw.run_chat(&sid, "hi", None, None).await;

    let msgs = claw.get_messages(&sid);
    let user_count = msgs.iter().filter(|m| m["role"] == "user").count();
    assert_eq!(
        user_count, 1,
        "user message should not be duplicated across agent reruns"
    );
    claw.stop().await.unwrap();
}

#[tokio::test]
async fn error_when_no_provider() {
    let dir = tempfile::tempdir().unwrap();
    // No mock, no API key → agent emits stream.error and returns empty.
    let claw = FastClaw::builder(FastClawConfig {
        workspace_root: Some(dir.path().to_path_buf()),
        ..FastClawConfig::default()
    })
    .build();
    claw.start().await.unwrap();
    let sid = claw.new_session(None);
    let reply = claw.run_chat(&sid, "hi", None, None).await;
    assert_eq!(reply, "");
    claw.stop().await.unwrap();
}

#[tokio::test]
async fn stream_chunk_events_captured() {
    let dir = tempfile::tempdir().unwrap();
    let (claw, _mock) = build_claw(dir.path(), vec![MockReply::Text("Hello world".into())]);
    claw.start().await.unwrap();

    let sid = claw.new_session(None);
    let events = common::collect_events(&claw, &sid, "hi").await;

    // Collects chunks and terminates with stream.end.
    let chunks: String = events
        .iter()
        .filter(|e| e.type_ == "stream.chunk")
        .filter_map(|e| e.payload.get("delta").and_then(|v| v.as_str()))
        .collect();
    assert_eq!(chunks, "Hello world");
    assert!(events.iter().any(|e| e.type_ == "stream.end"));
    claw.stop().await.unwrap();
}

#[tokio::test]
async fn thinking_events_emitted_and_persisted() {
    let dir = tempfile::tempdir().unwrap();
    let (claw, _mock) = build_claw(
        dir.path(),
        vec![MockReply::TextThinking {
            reasoning: "let me think".into(),
            text: "answer".into(),
            signature: None,
        }],
    );
    claw.start().await.unwrap();

    let sid = claw.new_session(None);
    let events = common::collect_events(&claw, &sid, "hi").await;

    let thinking: String = events
        .iter()
        .filter(|e| e.type_ == "stream.thinking")
        .filter_map(|e| e.payload.get("delta").and_then(|v| v.as_str()))
        .collect();
    assert_eq!(thinking, "let me think");

    // reasoning_content is persisted on the assistant message.
    let msgs = claw.get_messages(&sid);
    let assistant = msgs.iter().find(|m| m["role"] == "assistant").unwrap();
    assert_eq!(assistant["reasoning_content"], "let me think");
    claw.stop().await.unwrap();
}

#[tokio::test]
async fn thinking_signature_persisted() {
    let dir = tempfile::tempdir().unwrap();
    let (claw, _mock) = build_claw(
        dir.path(),
        vec![MockReply::TextThinking {
            reasoning: "deep think".into(),
            text: "answer".into(),
            signature: Some("sig-xyz".into()),
        }],
    );
    claw.start().await.unwrap();

    let sid = claw.new_session(None);
    let _ = claw.run_chat(&sid, "hi", None, None).await;

    let msgs = claw.get_messages(&sid);
    let assistant = msgs.iter().find(|m| m["role"] == "assistant").unwrap();
    assert_eq!(assistant["reasoning_content"], "deep think");
    assert_eq!(assistant["thinking_signature"], "sig-xyz");
    claw.stop().await.unwrap();
}

#[tokio::test]
async fn tool_result_event_emitted() {
    let dir = tempfile::tempdir().unwrap();
    let (claw, _mock) = build_claw(
        dir.path(),
        vec![
            MockReply::Tool(vec![(
                "run_shell".into(),
                "{\"command\":\"echo hi\"}".into(),
            )]),
            MockReply::Text("done".into()),
        ],
    );
    claw.start().await.unwrap();

    let sid = claw.new_session(None);
    let events = common::collect_events(&claw, &sid, "run something").await;

    let tool_results: Vec<&str> = events
        .iter()
        .filter(|e| e.type_ == "stream.tool_result")
        .filter_map(|e| e.payload.get("tool_name").and_then(|v| v.as_str()))
        .collect();
    assert_eq!(tool_results, vec!["run_shell"]);
    // The tool result event carries the tool output.
    let result = events
        .iter()
        .find(|e| e.type_ == "stream.tool_result")
        .and_then(|e| e.payload.get("result").and_then(|v| v.as_str()))
        .unwrap();
    assert!(result.contains("hi"));
    claw.stop().await.unwrap();
}

#[tokio::test]
async fn multiple_parallel_tool_calls() {
    let dir = tempfile::tempdir().unwrap();
    let (claw, _mock) = build_claw(
        dir.path(),
        vec![
            MockReply::Tool(vec![
                ("run_shell".into(), "{\"command\":\"echo a\"}".into()),
                ("run_shell".into(), "{\"command\":\"echo b\"}".into()),
            ]),
            MockReply::Text("finished".into()),
        ],
    );
    claw.start().await.unwrap();

    let sid = claw.new_session(None);
    let events = common::collect_events(&claw, &sid, "do two things").await;

    let tool_result_count = events
        .iter()
        .filter(|e| e.type_ == "stream.tool_result")
        .count();
    assert_eq!(tool_result_count, 2);

    // Both tool results become `role: "tool"` messages.
    let msgs = claw.get_messages(&sid);
    let tool_msgs: Vec<_> = msgs.iter().filter(|m| m["role"] == "tool").collect();
    assert_eq!(tool_msgs.len(), 2);
    claw.stop().await.unwrap();
}
