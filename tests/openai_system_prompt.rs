// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! End-to-end regression tests for the OpenAI gateway system-prompt fix.
//!
//! Commit `8088a09` ("增加system prompt注入接口-解决bug") fixed the OpenAI adapter
//! silently dropping `LlmRequest.system`: the host instructions (including
//! injected `PromptSection`s) never reached OpenAI-compatible gateways.
//!
//! These tests exercise the *real* `OpenAiProvider` — not a mock provider — by
//! pointing its `base_url` at a tiny local HTTP server that records the request
//! body. This locks the fix at the wire boundary for both `chat` (non-streaming)
//! and `chat_stream` (SSE), and verifies the injection feature reaches the body.

#![cfg(feature = "openai")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fastclaw::llm::openai::OpenAiProvider;
use fastclaw::llm::provider::{LlmProvider, LlmRequest, Message, NullSink, ThinkingPref};
use fastclaw::{FastClaw, FastClawConfig, PromptSection};
use serde_json::Value;

/// A minimal OpenAI-compatible HTTP server: accepts `requests` connections,
/// records each JSON body, and replies with either a JSON completion or an SSE
/// stream (decided by the request's `"stream"` field).
struct MockGateway {
    port: u16,
    captured: Arc<Mutex<Vec<Value>>>,
}

impl MockGateway {
    fn start(requests: usize, reply_text: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock gateway");
        let port = listener.local_addr().unwrap().port();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let sink = captured.clone();
        std::thread::spawn(move || {
            for _ in 0..requests {
                let Ok((stream, _)) = listener.accept() else {
                    break;
                };
                handle_connection(stream, reply_text, &sink);
            }
        });
        MockGateway { port, captured }
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.port)
    }

    fn requests(&self) -> Vec<Value> {
        self.captured.lock().unwrap().clone()
    }
}

fn handle_connection(mut stream: TcpStream, reply: &str, sink: &Mutex<Vec<Value>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));

    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some(v) = trimmed.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        let _ = reader.read_exact(&mut body);
    }
    let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let streaming = value
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // Record before replying so the client observing the response guarantees
    // the body is already visible to the test.
    sink.lock().unwrap().push(value);

    let response = if streaming {
        sse_response(reply)
    } else {
        json_response(reply)
    };
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Both);
}

fn json_response(text: &str) -> String {
    let body = serde_json::json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "created": 0,
        "model": "test-model",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": text},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
    })
    .to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
}

fn sse_response(text: &str) -> String {
    let delta = serde_json::json!({
        "id": "chatcmpl-test",
        "object": "chat.completion.chunk",
        "created": 0,
        "model": "test-model",
        "choices": [{
            "index": 0,
            "delta": {"role": "assistant", "content": text},
            "finish_reason": null
        }]
    });
    let stop = serde_json::json!({
        "id": "chatcmpl-test",
        "object": "chat.completion.chunk",
        "created": 0,
        "model": "test-model",
        "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]
    });
    let body = format!("data: {delta}\n\ndata: {stop}\n\ndata: [DONE]\n\n");
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
}

fn request(system: Option<&str>, messages: Vec<Message>) -> LlmRequest {
    LlmRequest {
        system: system.map(str::to_string),
        messages,
        tools: vec![],
        model: "test-model".into(),
        max_tokens: None,
        thinking: ThinkingPref::Adaptive,
    }
}

fn roles(body: &Value) -> Vec<String> {
    body["messages"]
        .as_array()
        .expect("messages array")
        .iter()
        .map(|m| m["role"].as_str().unwrap_or("").to_string())
        .collect()
}

// ── non-streaming `chat` ────────────────────────────────────────────────────

#[tokio::test]
async fn system_prompt_is_first_message_on_the_wire() {
    let gateway = MockGateway::start(1, "pong");
    let provider = OpenAiProvider::new(
        "test-key".into(),
        "test-model".into(),
        gateway.base_url(),
    );
    let req = request(Some("SYS-PROMPT"), vec![Message::user("hello")]);

    let resp = provider.chat(&req).await.unwrap();
    assert_eq!(resp.text, "pong");

    let bodies = gateway.requests();
    assert_eq!(bodies.len(), 1);
    let msgs = bodies[0]["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2, "system + user");
    assert_eq!(msgs[0]["role"], "system");
    assert_eq!(msgs[0]["content"], "SYS-PROMPT");
    assert_eq!(msgs[1]["role"], "user");
    assert_eq!(msgs[1]["content"], "hello");
}

#[tokio::test]
async fn blank_system_is_not_sent_nonstream() {
    for system in [None, Some(""), Some("   \n  ")] {
        let gateway = MockGateway::start(1, "ok");
        let provider =
            OpenAiProvider::new("test-key".into(), "test-model".into(), gateway.base_url());
        let req = request(system, vec![Message::user("hi")]);
        provider.chat(&req).await.unwrap();

        let bodies = gateway.requests();
        assert_eq!(roles(&bodies[0]), vec!["user"], "system={system:?}");
    }
}

#[tokio::test]
async fn system_prompt_is_trimmed_but_inner_layout_kept() {
    let gateway = MockGateway::start(1, "ok");
    let provider = OpenAiProvider::new("test-key".into(), "test-model".into(), gateway.base_url());
    let req = request(
        Some("\n  ## Host\nline1\nline2  \n"),
        vec![Message::user("hi")],
    );
    provider.chat(&req).await.unwrap();

    let sent = gateway.requests()[0]["messages"][0]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(sent, "## Host\nline1\nline2");
}

#[tokio::test]
async fn system_only_request_sends_single_message() {
    let gateway = MockGateway::start(1, "ok");
    let provider = OpenAiProvider::new("test-key".into(), "test-model".into(), gateway.base_url());
    let req = request(Some("only-system"), vec![]);
    provider.chat(&req).await.unwrap();

    let bodies = gateway.requests();
    assert_eq!(roles(&bodies[0]), vec!["system"]);
}

#[tokio::test]
async fn conversation_order_is_preserved_after_system() {
    let gateway = MockGateway::start(1, "ok");
    let provider = OpenAiProvider::new("test-key".into(), "test-model".into(), gateway.base_url());
    let req = request(
        Some("SYS"),
        vec![
            Message::user("u1"),
            Message::assistant("a1"),
            Message::user("u2"),
        ],
    );
    provider.chat(&req).await.unwrap();

    let bodies = gateway.requests();
    assert_eq!(roles(&bodies[0]), vec!["system", "user", "assistant", "user"]);
}

// ── streaming `chat_stream` ─────────────────────────────────────────────────

#[tokio::test]
async fn system_prompt_is_first_message_on_the_wire_streaming() {
    let gateway = MockGateway::start(1, "pong");
    let provider = OpenAiProvider::new("test-key".into(), "test-model".into(), gateway.base_url());
    let req = request(Some("SYS-STREAM"), vec![Message::user("hello")]);

    let mut sink = NullSink;
    let resp = provider.chat_stream(&req, &mut sink).await.unwrap();
    assert_eq!(resp.text, "pong");
    assert_eq!(resp.finish_reason.as_deref(), Some("stop"));

    let bodies = gateway.requests();
    let msgs = bodies[0]["messages"].as_array().unwrap();
    assert_eq!(msgs[0]["role"], "system");
    assert_eq!(msgs[0]["content"], "SYS-STREAM");
    assert_eq!(msgs[1]["role"], "user");
}

#[tokio::test]
async fn blank_system_is_not_sent_streaming() {
    let gateway = MockGateway::start(1, "ok");
    let provider = OpenAiProvider::new("test-key".into(), "test-model".into(), gateway.base_url());
    let req = request(Some("  "), vec![Message::user("hi")]);

    let mut sink = NullSink;
    provider.chat_stream(&req, &mut sink).await.unwrap();

    let bodies = gateway.requests();
    assert_eq!(roles(&bodies[0]), vec!["user"]);
}

// ── full host stack: FastClaw injection → real OpenAI request body ──────────

const MARK: &str = "MARKER-openai-9c1f-browser-guide";

#[tokio::test]
async fn fastclaw_injection_reaches_real_openai_request_body() {
    let gateway = MockGateway::start(1, "ok");
    let dir = tempfile::tempdir().unwrap();

    // Configure an agent whose OpenAI gateway is the local mock server.
    let agent_dir = dir.path().join("data/agents/oa");
    std::fs::create_dir_all(&agent_dir).unwrap();
    let metadata = serde_json::json!({
        "name": "oa",
        "llm": {
            "gateway": "openai",
            "model": "test-model",
            "api_key": "test-key",
            "base_url": gateway.base_url(),
        }
    });
    std::fs::write(agent_dir.join("metadata.json"), metadata.to_string()).unwrap();

    let claw = FastClaw::builder(FastClawConfig {
        workspace_root: Some(dir.path().to_path_buf()),
        ..FastClawConfig::default()
    })
    .build();
    claw.set_injections(vec![PromptSection::new(
        "浏览器能力（fastbrowser）",
        MARK,
    )]);
    claw.start().await.unwrap();

    let sid = claw.new_session(Some("oa"));
    let _ = claw.run_chat(&sid, "hi", None, None).await;
    claw.stop().await.unwrap();

    let bodies = gateway.requests();
    assert!(!bodies.is_empty(), "gateway should have been called");
    let msgs = bodies[0]["messages"].as_array().unwrap();
    assert_eq!(msgs[0]["role"], "system", "host system prompt must lead");
    let sys = msgs[0]["content"].as_str().unwrap();
    assert!(sys.contains("## 浏览器能力（fastbrowser）"), "missing title: {sys}");
    assert!(sys.contains(MARK), "missing injected content");
    assert!(
        msgs.iter().any(|m| m["role"] == "user"),
        "user turn must follow"
    );
}
