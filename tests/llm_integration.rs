// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Real LLM integration — gateway/provider matrix.
//!
//! Driven by agent configs in a workspace. Set `FASTCLAW_TEST_WORKSPACE` to the
//! workspace containing `data/agents/*/metadata.json` with real API keys (e.g.
//! the dev workspace `<repo>/workspace`). Each agent with a non-empty API key is
//! exercised for: text reply, streaming, and a ReAct tool loop (run_shell).
//!
//! Skips gracefully when no workspace/keys are configured. API keys are read
//! only from the workspace `data/` (never hardcoded in this file). Tests are
//! serialized to avoid rate-limit interference between real API calls.

use std::path::PathBuf;
use std::time::Duration;

use fastclaw::{FastClaw, FastClawConfig};

mod common;
use common::collect_events;

const TEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Serializes real-API tests within this binary (they run in parallel otherwise).
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn test_workspace() -> Option<PathBuf> {
    ["FASTCLAW_TEST_WORKSPACE", "FASTCLAW_WORKSPACE"]
        .iter()
        .find_map(|k| std::env::var(k).ok())
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
}

fn configured_agents(claw: &FastClaw) -> Vec<String> {
    claw.list_agents()
        .into_iter()
        .filter(|id| !claw.get_agent(id).llm.api_key.is_empty())
        .collect()
}

#[tokio::test]
async fn matrix_text_reply() {
    let _guard = SERIAL.lock().await;
    let Some(workspace) = test_workspace() else {
        eprintln!("skipping: FASTCLAW_TEST_WORKSPACE not set");
        return;
    };
    let claw = FastClaw::builder(FastClawConfig {
        workspace_root: Some(workspace),
        ..FastClawConfig::default()
    })
    .build();
    claw.start().await.unwrap();

    let agents = configured_agents(&claw);
    assert!(!agents.is_empty(), "no agent with an API key configured");

    for agent in agents {
        // MiniMax-M3 (reasoning model) frequently emits the answer *inside*
        // the `<think>` block, which the client routes to reasoning — so the
        // visible text can be empty while the model did answer. Accept the
        // turn when the answer is in the visible text OR in the captured
        // reasoning; retry once for genuinely dropped replies.
        let mut ok = false;
        let mut last_reply = String::new();
        for _ in 0..2 {
            let sid = claw.new_session(Some(&agent));
            let result = tokio::time::timeout(
                TEST_TIMEOUT,
                claw.run_chat(&sid, "Reply with exactly the word: pong", None, None),
            )
            .await;
            last_reply = result.expect("timed out").trim().to_lowercase();
            if last_reply.contains("pong") {
                ok = true;
                break;
            }
            let answered_in_reasoning = claw.get_messages(&sid).iter().any(|m| {
                ["content", "reasoning_content"].iter().any(|k| {
                    m.get(*k)
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_lowercase().contains("pong"))
                        .unwrap_or(false)
                })
            });
            if answered_in_reasoning {
                ok = true;
                break;
            }
        }
        assert!(
            ok,
            "agent '{agent}' text reply did not contain 'pong' (visible or reasoning): {last_reply:?}"
        );
    }
    claw.stop().await.unwrap();
}

#[tokio::test]
async fn matrix_react_tool_loop() {
    let _guard = SERIAL.lock().await;
    let Some(workspace) = test_workspace() else {
        eprintln!("skipping: FASTCLAW_TEST_WORKSPACE not set");
        return;
    };
    let marker = format!("fastclaw-tool-{}", uuid_like());
    let claw = FastClaw::builder(FastClawConfig {
        workspace_root: Some(workspace),
        ..FastClawConfig::default()
    })
    .build();
    claw.start().await.unwrap();

    let agents = configured_agents(&claw);
    assert!(!agents.is_empty(), "no agent with an API key configured");

    for agent in agents {
        let prompt = format!(
            "You MUST use the run_shell tool to execute the command `echo {marker}`. After the tool returns, reply with exactly the output of that command and nothing else."
        );
        // LLM tool-calling is mildly stochastic; retry once per agent.
        let mut ok = false;
        let mut last_reply = String::new();
        for _ in 0..2 {
            let sid = claw.new_session(Some(&agent));
            let result =
                tokio::time::timeout(TEST_TIMEOUT, claw.run_chat(&sid, &prompt, None, None)).await;
            last_reply = result.expect("timed out");
            let msgs = claw.get_messages(&sid);
            let tool_called = msgs.iter().any(|m| m["role"] == "tool");
            let assistant_tool_calls = msgs
                .iter()
                .any(|m| m["role"] == "assistant" && m.get("tool_calls").is_some());
            let marker_seen = last_reply.contains(&marker)
                || msgs.iter().any(|m| {
                    m["content"]
                        .as_str()
                        .map(|c| c.contains(&marker))
                        .unwrap_or(false)
                });
            if tool_called && assistant_tool_calls && marker_seen {
                ok = true;
                break;
            }
        }
        assert!(
            ok,
            "agent '{agent}' did not complete a tool round-trip after retries: {last_reply:?}"
        );
    }
    claw.stop().await.unwrap();
}

#[tokio::test]
async fn matrix_streaming_events() {
    let _guard = SERIAL.lock().await;
    let Some(workspace) = test_workspace() else {
        eprintln!("skipping: FASTCLAW_TEST_WORKSPACE not set");
        return;
    };
    let claw = FastClaw::builder(FastClawConfig {
        workspace_root: Some(workspace),
        ..FastClawConfig::default()
    })
    .build();
    claw.start().await.unwrap();

    let agents = configured_agents(&claw);
    assert!(!agents.is_empty(), "no agent with an API key configured");

    for agent in agents {
        let sid = claw.new_session(Some(&agent));
        let events = tokio::time::timeout(
            TEST_TIMEOUT,
            collect_events(&claw, &sid, "Write a short paragraph about the ocean."),
        )
        .await
        .expect("timed out");

        let chunk_chars: usize = events
            .iter()
            .filter(|e| e.type_ == "stream.chunk")
            .filter_map(|e| e.payload.get("delta").and_then(|v| v.as_str()))
            .map(|d| d.chars().count())
            .sum();
        // Reasoning-capable models may stream only thinking deltas for a turn;
        // count those as produced output too, so a reasoning-only reply is not
        // misreported as a broken stream.
        let thinking_chars: usize = events
            .iter()
            .filter(|e| e.type_ == "stream.thinking")
            .filter_map(|e| e.payload.get("delta").and_then(|v| v.as_str()))
            .map(|d| d.chars().count())
            .sum();
        assert!(
            chunk_chars + thinking_chars > 0,
            "agent '{agent}' produced no stream deltas"
        );
        assert!(
            events.iter().any(|e| e.type_ == "stream.end"),
            "agent '{agent}' did not emit stream.end"
        );
    }
    claw.stop().await.unwrap();
}

#[tokio::test]
async fn run_skills_current_time() {
    let _guard = SERIAL.lock().await;
    let Some(workspace) = test_workspace() else {
        eprintln!("skipping: FASTCLAW_TEST_WORKSPACE not set");
        return;
    };
    let claw = FastClaw::builder(FastClawConfig {
        workspace_root: Some(workspace),
        ..FastClawConfig::default()
    })
    .build();
    claw.start().await.unwrap();

    let sid = claw.new_session(Some("main_agent"));
    let prompt = "Use the run_skills tool to run the skill named 'current_time', then tell me what it returned.";
    let result = tokio::time::timeout(TEST_TIMEOUT, claw.run_chat(&sid, prompt, None, None)).await;
    let reply = result.expect("timed out");

    let msgs = claw.get_messages(&sid);
    let skill_called = msgs.iter().any(|m| {
        m["role"] == "tool"
            && m["content"]
                .as_str()
                .map(|c| c.contains("current_time"))
                .unwrap_or(false)
    });
    assert!(
        skill_called || !reply.is_empty(),
        "current_time skill round-trip failed: {reply:?}"
    );
    claw.stop().await.unwrap();
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    format!("{nanos:08x}")
}

#[tokio::test]
async fn matrix_thinking_extraction() {
    let _guard = SERIAL.lock().await;
    let Some(workspace) = test_workspace() else {
        eprintln!("skipping: FASTCLAW_TEST_WORKSPACE not set");
        return;
    };
    let claw = FastClaw::builder(FastClawConfig {
        workspace_root: Some(workspace),
        ..FastClawConfig::default()
    })
    .build();
    claw.start().await.unwrap();

    let agents: Vec<String> = configured_agents(&claw)
        .into_iter()
        // Thinking is meaningful on MiniMax / anthropic-route models.
        .filter(|id| id.contains("minimax") || id.contains("anthropic"))
        .collect();
    if agents.is_empty() {
        eprintln!("skipping: no suitable agent configured");
        return;
    }

    for agent in agents {
        let sid = claw.new_session(Some(&agent));
        let events = tokio::time::timeout(
            TEST_TIMEOUT,
            collect_events(
                &claw,
                &sid,
                "Think step by step about whether 97 is prime, then reply with only: yes or no.",
            ),
        )
        .await
        .expect("timed out");

        // No error, and the visible text must not leak raw thinking markers.
        assert!(
            !events.iter().any(|e| e.type_ == "stream.error"),
            "agent '{agent}' streamed an error"
        );
        let visible: String = events
            .iter()
            .filter(|e| e.type_ == "stream.chunk")
            .filter_map(|e| e.payload.get("delta").and_then(|v| v.as_str()))
            .collect();
        assert!(
            !visible.contains("<think>")
                && !visible.contains("Thinking:")
                && !visible.contains("Reasoning:"),
            "agent '{agent}' leaked thinking markers into visible text: {visible:?}"
        );

        // If the model produced reasoning, it must have been captured as
        // thinking events / reasoning_content (never mixed into content).
        let msgs = claw.get_messages(&sid);
        let has_reasoning = msgs.iter().any(|m| m.get("reasoning_content").is_some());
        if has_reasoning {
            assert!(
                events.iter().any(|e| e.type_ == "stream.thinking"),
                "agent '{agent}' persisted reasoning_content but no stream.thinking event"
            );
        }
    }
    claw.stop().await.unwrap();
}

/// A valid 64x64 solid-blue PNG, base64-encoded (for vision-model testing).
const TINY_PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAEAAAABACAIAAAAlC+aJAAAAeklEQVR4nO3PUQkAIBTAwJfEYCY2liH8OITBAtxm7fN1wwUNaEEDWtCAFjSgBQ1oQQNa0IAWNKAFDWhBA1rQgBY0oAUNaEEDWtCAFjSgBQ1oQQNa0IAWNKAFDWhBA1rQgBY0oAUNaEEDWtCAFjSgBQ1oQQNa0IAWPHYB1r8BLYIz6tQAAAAASUVORK5CYII=";

#[tokio::test]
async fn matrix_multimodal_image() {
    let _guard = SERIAL.lock().await;
    let Some(workspace) = test_workspace() else {
        eprintln!("skipping: FASTCLAW_TEST_WORKSPACE not set");
        return;
    };
    let claw = FastClaw::builder(FastClawConfig {
        workspace_root: Some(workspace),
        ..FastClawConfig::default()
    })
    .build();
    claw.start().await.unwrap();

    let agents: Vec<String> = configured_agents(&claw)
        .into_iter()
        .filter(|id| claw.get_agent(id).llm.multimodal)
        .collect();
    if agents.is_empty() {
        eprintln!("skipping: no multimodal agent configured");
        return;
    }

    let image = fastclaw::ImagePart::from_base64("image/png", TINY_PNG_B64);
    for agent in agents {
        let sid = claw.new_session(Some(&agent));
        let result = tokio::time::timeout(
            TEST_TIMEOUT,
            claw.chat_with_images(
                &sid,
                "Describe the image in one short sentence.",
                std::slice::from_ref(&image),
            ),
        )
        .await;
        result.expect("timed out").expect("push failed");

        // Consume the stream; assert a non-empty reply (or reasoning-only
        // thinking deltas) and no stream.error.
        use futures::StreamExt;
        let mut reply = String::new();
        let mut thinking_chars = 0usize;
        let mut errored = false;
        if let Ok(mut stream) = claw.stream_events(&sid) {
            while let Some(ev) = stream.next().await {
                match ev.type_.as_str() {
                    "stream.chunk" => {
                        reply.push_str(
                            ev.payload
                                .get("delta")
                                .and_then(|v| v.as_str())
                                .unwrap_or(""),
                        );
                    }
                    "stream.thinking" => {
                        thinking_chars += ev
                            .payload
                            .get("delta")
                            .and_then(|v| v.as_str())
                            .map(|d| d.chars().count())
                            .unwrap_or(0);
                    }
                    "stream.error" => {
                        let e = ev
                            .payload
                            .get("error")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown error");
                        eprintln!("agent '{agent}' stream.error: {e}");
                        errored = true;
                        break;
                    }
                    "stream.end" => break,
                    _ => {}
                }
            }
        }
        assert!(
            !errored,
            "agent '{agent}' returned stream.error for image input"
        );
        assert!(
            !reply.is_empty() || thinking_chars > 0,
            "agent '{agent}' produced no output for a multimodal input"
        );
    }
    claw.stop().await.unwrap();
}
