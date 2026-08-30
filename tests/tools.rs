// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! `run_skills` tool + `ToolContext::execute` dispatch.

use std::sync::Arc;

use fastclaw::tools::skills::run_skills;
use fastclaw::{Error, Skill, State};
use serde_json::{json, Value};

mod common;
use common::make_tool_context;

struct EchoSkill;

#[async_trait::async_trait]
impl Skill for EchoSkill {
    fn name(&self) -> &str {
        "echo_skill"
    }
    fn description(&self) -> &str {
        "Echo back the msg param"
    }
    async fn execute(&self, params: Value) -> fastclaw::Result<String> {
        Ok(params
            .get("msg")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string())
    }
}

#[tokio::test]
async fn list_empty() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = make_tool_context(dir.path());
    let out = run_skills(&ctx, Some("__list__".into()), None, None)
        .await
        .unwrap();
    assert!(out.contains("No skills available"));
}

#[tokio::test]
async fn list_shows_native() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = make_tool_context(dir.path());
    ctx.skills.register_native(Arc::new(EchoSkill));
    let out = run_skills(&ctx, Some("__list__".into()), None, None)
        .await
        .unwrap();
    assert!(out.contains("echo_skill"));
    assert!(out.contains("Echo back the msg param"));
}

#[tokio::test]
async fn execute_native() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = make_tool_context(dir.path());
    ctx.skills.register_native(Arc::new(EchoSkill));
    let out = run_skills(
        &ctx,
        Some("echo_skill".into()),
        Some(json!({ "msg": "hello" })),
        None,
    )
    .await
    .unwrap();
    assert_eq!(out, "hello");
}

#[tokio::test]
async fn info_requires_skill_name() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = make_tool_context(dir.path());
    let out = run_skills(&ctx, Some("__info__".into()), Some(json!({})), None)
        .await
        .unwrap();
    assert!(out.contains("skill_name is required"));
}

#[tokio::test]
async fn not_found_error() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = make_tool_context(dir.path());
    let out = run_skills(&ctx, Some("nope".into()), None, None).await;
    assert!(out.is_err());
}

#[tokio::test]
async fn tool_context_execute_run_shell() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = make_tool_context(dir.path());
    let mut state = State::new();
    state.set_session_id("s1");
    let out = ctx
        .execute(
            "run_shell",
            &mut state,
            json!({ "command": "echo dispatch-ok" }),
        )
        .await
        .unwrap();
    assert!(out.as_str().unwrap().contains("dispatch-ok"));
}

#[tokio::test]
async fn tool_context_execute_missing_command() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = make_tool_context(dir.path());
    let mut state = State::new();
    state.set_session_id("s1");
    let out = ctx
        .execute("run_shell", &mut state, json!({}))
        .await
        .unwrap();
    assert!(out.as_str().unwrap().contains("required"));
}

#[tokio::test]
async fn tool_context_execute_unknown_tool() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = make_tool_context(dir.path());
    let mut state = State::new();
    state.set_session_id("s1");
    let err = ctx.execute("no_such_tool", &mut state, json!({})).await;
    assert!(matches!(err, Err(Error::NotFound(_))));
}
