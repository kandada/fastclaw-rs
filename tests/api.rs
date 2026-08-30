// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! FastClaw facade: sessions, settings, workdirs, skills.

use std::sync::Arc;

use fastclaw::{FastClaw, FastClawConfig};

mod common;
use common::{build_claw, MockProvider, MockReply};

#[test]
fn build_and_start_without_llm() {
    let dir = tempfile::tempdir().unwrap();
    let claw = FastClaw::builder(FastClawConfig {
        workspace_root: Some(dir.path().to_path_buf()),
        ..FastClawConfig::default()
    })
    .build();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        claw.start().await.unwrap();
        assert!(claw.is_running());
        claw.stop().await.unwrap();
        assert!(!claw.is_running());
    });
}

#[tokio::test]
async fn session_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let claw = FastClaw::builder(FastClawConfig {
        workspace_root: Some(dir.path().to_path_buf()),
        ..FastClawConfig::default()
    })
    .build();

    let sid = claw.new_session(Some("main_agent"));
    assert!(claw.get_session(&sid).is_some());
    assert_eq!(claw.list_sessions().len(), 1);

    // New session is distinct.
    let sid2 = claw.new_session(None);
    assert_ne!(sid, sid2);
    assert_eq!(claw.list_sessions().len(), 2);

    claw.delete_session(&sid).await;
    assert!(claw.get_session(&sid).is_none());
    assert_eq!(claw.list_sessions().len(), 1);
}

#[tokio::test]
async fn workdir_binding_and_resolution() {
    let dir = tempfile::tempdir().unwrap();
    let claw = FastClaw::builder(FastClawConfig {
        workspace_root: Some(dir.path().to_path_buf()),
        ..FastClawConfig::default()
    })
    .build();

    let sid = claw.new_session(None);
    let default = claw.default_work_dir();

    // Binding changes resolution.
    let bound = dir.path().join("bound-dir");
    claw.bind_work_dir(&sid, &bound).unwrap();
    assert_eq!(claw.resolve_work_dir(&sid, None), bound);

    claw.unbind_work_dir(&sid);
    assert_eq!(claw.resolve_work_dir(&sid, None), default);

    // Register / list / default.
    let extra = dir.path().join("extra");
    claw.register_work_dir(&extra).unwrap();
    assert!(claw.list_work_dirs().contains(&extra));
    claw.set_default_work_dir(&extra).unwrap();
    assert_eq!(claw.default_work_dir(), extra);
}

#[tokio::test]
async fn skills_and_agents_listing() {
    let dir = tempfile::tempdir().unwrap();
    let claw = FastClaw::builder(FastClawConfig {
        workspace_root: Some(dir.path().to_path_buf()),
        ..FastClawConfig::default()
    })
    .build();

    // The built-in native `current_time` skill is registered.
    let skills = claw.list_skills();
    assert!(skills.iter().any(|s| s.name == "current_time"));
}

#[tokio::test]
async fn host_tool_registered_and_usable() {
    use serde_json::json;
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();

    let double_tool = fastclaw::fastmind::Tool::new(
        "double",
        "double a number",
        Arc::new(|args, _ctx| {
            Box::pin(async move {
                let n = args.get("n").and_then(|v| v.as_i64()).unwrap_or(0);
                Ok(json!(n * 2))
            })
        }),
    );

    let claw = FastClaw::builder(FastClawConfig {
        workspace_root: Some(dir.path().to_path_buf()),
        ..FastClawConfig::default()
    })
    .tool(double_tool)
    .build();
    let mock = MockProvider::new(vec![
        MockReply::Tool(vec![("double".into(), "{\"n\":21}".into())]),
        MockReply::Text("answer".into()),
    ]);
    claw.set_mock_provider(mock);
    claw.start().await.unwrap();

    let sid = claw.new_session(None);
    let _ = claw.run_chat(&sid, "double 21", None, None).await;

    // The host tool was invoked and produced a `tool` message with the result 42.
    let msgs = claw.get_messages(&sid);
    let tool_msg = msgs
        .iter()
        .find(|m| m["role"] == "tool")
        .expect("tool message");
    assert!(tool_msg["content"].as_str().unwrap().contains("42"));
    claw.stop().await.unwrap();
}

#[test]
fn ffm_style_stream_event_flattening() {
    let e = fastclaw::Event::new("stream.chunk", serde_json::json!({"delta": "hi"}), "s1");
    let flat = fastclaw::StreamEvent::from_event(&e);
    assert_eq!(flat.type_, "stream.chunk");
    assert_eq!(flat.session_id, "s1");
    assert!(flat.payload.contains("hi"));
}

#[tokio::test]
async fn settings_update_persists() {
    let dir = tempfile::tempdir().unwrap();
    let claw = FastClaw::builder(FastClawConfig {
        workspace_root: Some(dir.path().to_path_buf()),
        ..FastClawConfig::default()
    })
    .build();

    let mut s = claw.get_settings();
    s.run_shell_timeout = 123;
    claw.update_settings(s);

    let loaded = fastclaw::config::load_settings(claw.paths());
    assert_eq!(loaded.run_shell_timeout, 123);
}

#[tokio::test]
async fn clear_messages_removes_history() {
    let dir = tempfile::tempdir().unwrap();
    let claw = FastClaw::builder(FastClawConfig {
        workspace_root: Some(dir.path().to_path_buf()),
        ..FastClawConfig::default()
    })
    .build();
    let mock = MockProvider::new(vec![MockReply::Text("hi".into())]);
    claw.set_mock_provider(mock);
    claw.start().await.unwrap();

    let sid = claw.new_session(None);
    let _ = claw.run_chat(&sid, "hello", None, None).await;
    assert!(!claw.get_messages(&sid).is_empty());

    claw.clear_messages(&sid);
    assert!(claw.get_messages(&sid).is_empty());
    claw.stop().await.unwrap();
}

#[tokio::test]
async fn run_chat_invokes_callbacks() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    let dir = tempfile::tempdir().unwrap();
    let (claw, _mock) = build_claw(dir.path(), vec![MockReply::Text("abcd".into())]);
    claw.start().await.unwrap();

    let sid = claw.new_session(None);
    let chunk_calls = Arc::new(AtomicUsize::new(0));
    let end_text = Arc::new(Mutex::new(String::new()));

    let chunk_calls2 = chunk_calls.clone();
    let end_text2 = end_text.clone();
    let reply = claw
        .run_chat(
            &sid,
            "hi",
            Some(Arc::new(move |_d: String| {
                chunk_calls2.fetch_add(1, Ordering::SeqCst);
            })),
            Some(Arc::new(move |full: String| {
                *end_text2.lock().unwrap() = full;
            })),
        )
        .await;

    assert_eq!(reply, "abcd");
    assert_eq!(chunk_calls.load(Ordering::SeqCst), 4); // char-level streaming
    assert_eq!(*end_text.lock().unwrap(), "abcd");
    claw.stop().await.unwrap();
}

#[tokio::test]
async fn delete_session_cleans_disk() {
    let dir = tempfile::tempdir().unwrap();
    let claw = FastClaw::builder(FastClawConfig {
        workspace_root: Some(dir.path().to_path_buf()),
        ..FastClawConfig::default()
    })
    .build();
    let mock = MockProvider::new(vec![MockReply::Text("hi".into())]);
    claw.set_mock_provider(mock);
    claw.start().await.unwrap();

    let sid = claw.new_session(None);
    let _ = claw.run_chat(&sid, "hi", None, None).await;
    // The session dir exists on disk after message persistence.
    assert!(claw.paths().session_dir(&sid).exists());

    claw.delete_session(&sid).await;
    assert!(!claw.paths().session_dir(&sid).exists());
    assert!(claw.get_session(&sid).is_none());
    claw.stop().await.unwrap();
}
