// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! `run_shell` execution (real subprocess).

use fastclaw::tools::shell::run_shell;
use fastclaw::State;

mod common;
use common::make_tool_context;

fn make_state() -> State {
    let mut s = State::new();
    s.set_session_id("s1");
    s
}

#[tokio::test]
async fn echo_output() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = make_tool_context(dir.path());
    let out = run_shell(
        &ctx,
        "s1",
        &[],
        &mut make_state(),
        "echo hello-world".into(),
        Some(1000),
        Some(10),
    )
    .await
    .unwrap();
    assert!(out.contains("hello-world"));
}

#[tokio::test]
async fn truncates_output() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = make_tool_context(dir.path());
    let out = run_shell(
        &ctx,
        "s1",
        &[],
        &mut make_state(),
        "echo 0123456789".into(),
        Some(5),
        Some(10),
    )
    .await
    .unwrap();
    assert!(out.contains("truncated"));
}

#[tokio::test]
async fn timeout_kills_command() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = make_tool_context(dir.path());
    let out = run_shell(
        &ctx,
        "s1",
        &[],
        &mut make_state(),
        "sleep 5".into(),
        Some(1000),
        Some(1),
    )
    .await
    .unwrap();
    assert!(out.contains("timed out"));
}

#[tokio::test]
async fn cwd_is_working_directory() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("proj");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("marker.txt"), "x").unwrap();

    let ctx = make_tool_context(dir.path());
    ctx.workdirs.bind("s1", &work).unwrap();

    let out = run_shell(
        &ctx,
        "s1",
        &[],
        &mut make_state(),
        "ls".into(),
        Some(1000),
        Some(10),
    )
    .await
    .unwrap();
    assert!(out.contains("marker.txt"));
}

#[tokio::test]
async fn ask_user_gates_then_confirm_allows() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = make_tool_context(dir.path());

    // First attempt: gated (shutdown).
    let mut state = make_state();
    let out = run_shell(
        &ctx,
        "s1",
        &[],
        &mut state,
        "shutdown -h now".into(),
        Some(1000),
        Some(10),
    )
    .await
    .unwrap();
    assert!(out.starts_with("AskUser:"));

    // Confirmed: allowed.
    let out = run_shell(
        &ctx,
        "s1",
        &[],
        &mut state,
        "CONFIRM: echo ok".into(),
        Some(1000),
        Some(10),
    )
    .await
    .unwrap();
    assert!(out.contains("ok"));
}

#[tokio::test]
async fn deny_blocks_rm_rf_root() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = make_tool_context(dir.path());
    let out = run_shell(
        &ctx,
        "s1",
        &[],
        &mut make_state(),
        "rm -rf /".into(),
        Some(1000),
        Some(10),
    )
    .await
    .unwrap();
    assert!(out.contains("Permission denied"));
}

#[tokio::test]
async fn captures_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = make_tool_context(dir.path());
    let out = run_shell(
        &ctx,
        "s1",
        &[],
        &mut make_state(),
        "echo err-msg 1>&2".into(),
        Some(1000),
        Some(10),
    )
    .await
    .unwrap();
    assert!(out.contains("err-msg"));
}

#[tokio::test]
async fn no_output_message() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = make_tool_context(dir.path());
    let out = run_shell(
        &ctx,
        "s1",
        &[],
        &mut make_state(),
        "true".into(),
        Some(1000),
        Some(10),
    )
    .await
    .unwrap();
    assert!(out.contains("no output"));
}

#[tokio::test]
async fn full_output_when_max_length_negative() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = make_tool_context(dir.path());
    let out = run_shell(
        &ctx,
        "s1",
        &[],
        &mut make_state(),
        "printf 'a%.0s' {1..2000}".into(),
        Some(-1),
        Some(10),
    )
    .await
    .unwrap();
    assert_eq!(out.trim().len(), 2000);
    assert!(!out.contains("truncated"));
}

#[tokio::test]
async fn repeated_ask_reports_already_attempted() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = make_tool_context(dir.path());
    let mut state = make_state();
    let first = run_shell(
        &ctx,
        "s1",
        &[],
        &mut state,
        "shutdown -h now".into(),
        Some(1000),
        Some(10),
    )
    .await
    .unwrap();
    assert!(first.starts_with("AskUser:"));
    let second = run_shell(
        &ctx,
        "s1",
        &[],
        &mut state,
        "shutdown -h now".into(),
        Some(1000),
        Some(10),
    )
    .await
    .unwrap();
    assert!(second.contains("already attempted"));
}

#[tokio::test]
async fn nested_workdir_path_allowed() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("proj");
    std::fs::create_dir_all(work.join("sub")).unwrap();
    std::fs::write(work.join("sub").join("f.txt"), "x").unwrap();

    let ctx = make_tool_context(dir.path());
    ctx.workdirs.bind("s1", &work).unwrap();

    let nested = work
        .join("sub")
        .join("f.txt")
        .to_string_lossy()
        .into_owned();
    let out = run_shell(
        &ctx,
        "s1",
        &[],
        &mut make_state(),
        format!("cat {nested}"),
        Some(1000),
        Some(10),
    )
    .await
    .unwrap();
    assert!(out.contains("x"));
}
