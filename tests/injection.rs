// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Dedicated tests for the system-prompt injection API
//! (`set_injections` / `append_injection` / `injections`).
//!
//! Covers: in-memory semantics, instance/Profile isolation, non-persistence,
//! thread safety, and end-to-end application into the provider system prompt
//! (per-round read, no session rebind).

use std::path::Path;
use std::sync::Arc;

use fastclaw::{FastClaw, FastClawConfig, PromptSection};

mod common;
use common::{build_claw, MockProvider, MockReply};

fn build(root: &Path) -> FastClaw {
    FastClaw::builder(FastClawConfig {
        workspace_root: Some(root.to_path_buf()),
        ..FastClawConfig::default()
    })
    .build()
}

fn sec(title: &str, content: &str) -> PromptSection {
    PromptSection::new(title, content)
}

// Unique markers used as injection content. They are long and specific enough
// that they can never appear in the random tempdir paths that get substituted
// into `{workspace_path}` / `{workdir}` (short strings like "c1" / "v1" / "guide"
// randomly match path segments and make `find()` order assertions flaky).
const MARK_A: &str = "MARKER-7f3a-host-section-one";
const MARK_B: &str = "MARKER-7f3a-host-section-two";

/// The system prompt captured by the mock provider for the `i`-th LLM call.
fn system_of(mock: &MockProvider, i: usize) -> String {
    let calls = mock.calls.lock().unwrap();
    calls[i].system.as_deref().unwrap_or("").to_string()
}

// ── in-memory semantics ─────────────────────────────────────────────────────

#[test]
fn default_has_no_injections() {
    let dir = tempfile::tempdir().unwrap();
    let claw = build(dir.path());
    assert!(claw.injections().is_empty());
}

#[test]
fn append_preserves_order() {
    let dir = tempfile::tempdir().unwrap();
    let claw = build(dir.path());
    claw.append_injection(sec("t1", "c1"));
    claw.append_injection(sec("t2", "c2"));
    claw.append_injection(sec("t3", "c3"));
    let list = claw.injections();
    let titles: Vec<&str> = list.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, vec!["t1", "t2", "t3"]);
}

#[test]
fn set_replaces_entire_list() {
    let dir = tempfile::tempdir().unwrap();
    let claw = build(dir.path());
    claw.set_injections(vec![sec("a", "1"), sec("b", "2")]);
    claw.set_injections(vec![sec("c", "3")]);
    let list = claw.injections();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].title, "c");
    assert_eq!(list[0].content, "3");
}

#[test]
fn append_after_set_appends_to_tail() {
    let dir = tempfile::tempdir().unwrap();
    let claw = build(dir.path());
    claw.set_injections(vec![sec("a", "1")]);
    claw.append_injection(sec("b", "2"));
    let list = claw.injections();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].title, "a");
    assert_eq!(list[1].title, "b");
}

#[test]
fn set_empty_clears() {
    let dir = tempfile::tempdir().unwrap();
    let claw = build(dir.path());
    claw.set_injections(vec![sec("a", "1")]);
    claw.set_injections(vec![]);
    assert!(claw.injections().is_empty());
}

// ── isolation & lifetime ────────────────────────────────────────────────────

#[test]
fn injections_are_instance_local() {
    let dir = tempfile::tempdir().unwrap();
    // Two FastClaw instances sharing a workspace: injections must not leak
    // between them (mirrors "one Profile = one FastClaw" isolation).
    let claw_a = build(dir.path());
    let claw_b = build(dir.path());
    claw_a.set_injections(vec![sec("a", "1")]);
    claw_b.set_injections(vec![sec("b", "2")]);
    assert_eq!(claw_a.injections()[0].title, "a");
    assert_eq!(claw_b.injections()[0].title, "b");
}

#[tokio::test]
async fn injections_are_memory_only_not_persisted() {
    let dir = tempfile::tempdir().unwrap();
    let injection = sec("cap", "guide");
    {
        let claw = build(dir.path());
        claw.set_injections(vec![injection.clone()]);
        assert_eq!(claw.injections(), vec![injection.clone()]);
    } // dropped — simulates process restart
    let claw2 = build(dir.path());
    assert!(
        claw2.injections().is_empty(),
        "injections are in-memory and must be re-injected by the host after restart"
    );
}

#[test]
fn serde_roundtrip() {
    let sections = vec![sec("t1", "c1"), sec("t2", "c2")];
    let json = serde_json::to_string(&sections).unwrap();
    let back: Vec<PromptSection> = serde_json::from_str(&json).unwrap();
    assert_eq!(back, sections);
}

// ── thread safety ───────────────────────────────────────────────────────────

#[test]
fn concurrent_append_is_thread_safe() {
    let dir = tempfile::tempdir().unwrap();
    let claw = Arc::new(build(dir.path()));
    let handles: Vec<_> = (0..8)
        .map(|t| {
            let claw = claw.clone();
            std::thread::spawn(move || {
                for i in 0..5 {
                    claw.append_injection(sec(&format!("t{t}-{i}"), "x"));
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(claw.injections().len(), 8 * 5);
}

// ── application into the provider system prompt ─────────────────────────────

#[tokio::test]
async fn injection_reaches_provider_system_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let (claw, mock) = build_claw(dir.path(), vec![MockReply::Text("ok".into())]);
    claw.set_injections(vec![sec(
        "浏览器能力（fastbrowser）",
        "你有浏览器内核 fastbrowser。",
    )]);
    claw.start().await.unwrap();

    let sid = claw.new_session(None);
    let _ = claw.run_chat(&sid, "hi", None, None).await;

    let sys = system_of(&mock, 0);
    assert!(sys.contains("## 浏览器能力（fastbrowser）"));
    assert!(sys.contains("你有浏览器内核 fastbrowser。"));
    claw.stop().await.unwrap();
}

#[tokio::test]
async fn runtime_change_applies_to_next_request_without_rebind() {
    let dir = tempfile::tempdir().unwrap();
    let (claw, mock) = build_claw(
        dir.path(),
        vec![MockReply::Text("one".into()), MockReply::Text("two".into())],
    );
    claw.start().await.unwrap();
    let sid = claw.new_session(None);

    claw.set_injections(vec![sec("version", MARK_A)]);
    let _ = claw.run_chat(&sid, "msg one", None, None).await;
    let sys1 = system_of(&mock, 0);
    assert!(sys1.contains(MARK_A));

    // Change the injection at runtime; the next request on the SAME session
    // picks it up (per-round read — no rebind, no personality reload).
    claw.set_injections(vec![sec("version", MARK_B)]);
    let _ = claw.run_chat(&sid, "msg two", None, None).await;
    let sys2 = system_of(&mock, 1);
    assert!(sys2.contains(MARK_B));
    assert!(
        !sys2.contains(MARK_A),
        "stale injection must not leak into next round"
    );
    claw.stop().await.unwrap();
}

#[tokio::test]
async fn injection_applies_to_all_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let (claw, mock) = build_claw(
        dir.path(),
        vec![MockReply::Text("a".into()), MockReply::Text("b".into())],
    );
    claw.set_injections(vec![sec("cap", MARK_A)]);
    claw.start().await.unwrap();

    let s1 = claw.new_session(None);
    let s2 = claw.new_session(None);
    let _ = claw.run_chat(&s1, "hi s1", None, None).await;
    let _ = claw.run_chat(&s2, "hi s2", None, None).await;

    assert!(system_of(&mock, 0).contains(MARK_A));
    assert!(system_of(&mock, 1).contains(MARK_A));
    claw.stop().await.unwrap();
}

#[tokio::test]
async fn injection_present_in_every_react_round() {
    let dir = tempfile::tempdir().unwrap();
    let (claw, mock) = build_claw(
        dir.path(),
        vec![
            MockReply::Tool(vec![(
                "run_shell".into(),
                "{\"command\":\"echo hi\"}".into(),
            )]),
            MockReply::Text("done".into()),
        ],
    );
    claw.set_injections(vec![sec("cap", MARK_A)]);
    claw.start().await.unwrap();

    let sid = claw.new_session(None);
    let reply = claw.run_chat(&sid, "do it", None, None).await;
    assert_eq!(reply, "done");

    // Both the initial round and the round after the tool result carry the
    // injection (the system prompt is rebuilt per round).
    assert_eq!(mock.call_count(), 2);
    assert!(system_of(&mock, 0).contains(MARK_A));
    assert!(system_of(&mock, 1).contains(MARK_A));
    claw.stop().await.unwrap();
}

// ── rendering order & skipping ──────────────────────────────────────────────

#[tokio::test]
async fn multi_sections_render_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let (claw, mock) = build_claw(dir.path(), vec![MockReply::Text("ok".into())]);
    claw.set_injections(vec![sec("t1", MARK_A), sec("t2", MARK_B)]);
    claw.start().await.unwrap();

    let sid = claw.new_session(None);
    let _ = claw.run_chat(&sid, "hi", None, None).await;

    let sys = system_of(&mock, 0);
    let i1 = sys.find(MARK_A).expect("t1 content present");
    let i2 = sys.find(MARK_B).expect("t2 content present");
    assert!(i1 < i2, "injection sections must keep insertion order");
    claw.stop().await.unwrap();
}

#[tokio::test]
async fn empty_or_whitespace_sections_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let (claw, mock) = build_claw(dir.path(), vec![MockReply::Text("ok".into())]);
    claw.set_injections(vec![
        sec("t1", "real content"),
        sec("", ""),      // empty → skipped
        sec("t2", "   "), // whitespace-only → skipped
    ]);
    claw.start().await.unwrap();

    let sid = claw.new_session(None);
    let _ = claw.run_chat(&sid, "hi", None, None).await;

    let sys = system_of(&mock, 0);
    assert!(sys.contains("## t1"));
    assert!(sys.contains("real content"));
    assert!(
        !sys.contains("## t2"),
        "whitespace-only section must be skipped"
    );
    claw.stop().await.unwrap();
}

#[tokio::test]
async fn personality_comes_before_injections() {
    let dir = tempfile::tempdir().unwrap();
    let (claw, mock) = build_claw(dir.path(), vec![MockReply::Text("ok".into())]);
    claw.set_injections(vec![sec("宿主能力", "你有浏览器内核 fastbrowser。")]);
    claw.start().await.unwrap();

    let sid = claw.new_session(None);
    let _ = claw.run_chat(&sid, "hi", None, None).await;

    let sys = system_of(&mock, 0);
    let soul = sys
        .find("SOUL.md — Personality")
        .expect("personality present");
    let inj = sys.find("## 宿主能力").expect("injection present");
    assert!(soul < inj, "personality must precede host injections");
    claw.stop().await.unwrap();
}

// ── content fidelity ────────────────────────────────────────────────────────

/// Injections are appended *after* the template substitution pass, so any
/// placeholder-looking text inside an injection must survive verbatim.
#[tokio::test]
async fn injection_content_is_not_placeholder_substituted() {
    let dir = tempfile::tempdir().unwrap();
    let (claw, mock) = build_claw(dir.path(), vec![MockReply::Text("ok".into())]);
    claw.set_injections(vec![sec(
        "literal",
        "keep {session_id} {workdir} {skills_list} {workspace_path} verbatim",
    )]);
    claw.start().await.unwrap();

    let sid = claw.new_session(None);
    let _ = claw.run_chat(&sid, "hi", None, None).await;

    let sys = system_of(&mock, 0);
    assert!(
        sys.contains("keep {session_id} {workdir} {skills_list} {workspace_path} verbatim"),
        "injection content must not be placeholder-substituted: {sys}"
    );
    claw.stop().await.unwrap();
}

#[tokio::test]
async fn title_and_content_are_trimmed_in_render() {
    let dir = tempfile::tempdir().unwrap();
    let (claw, mock) = build_claw(dir.path(), vec![MockReply::Text("ok".into())]);
    claw.set_injections(vec![sec("  spaced title  ", "\n  body line  \n")]);
    claw.start().await.unwrap();

    let sid = claw.new_session(None);
    let _ = claw.run_chat(&sid, "hi", None, None).await;

    let sys = system_of(&mock, 0);
    assert!(
        sys.contains("## spaced title\nbody line"),
        "title/content must be trimmed: {sys}"
    );
    assert!(!sys.contains("##   spaced title"), "untrimmed title leaked");
    claw.stop().await.unwrap();
}

#[tokio::test]
async fn empty_title_renders_bare_content_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let (claw, mock) = build_claw(dir.path(), vec![MockReply::Text("ok".into())]);
    claw.set_injections(vec![sec("", "just a note")]);
    claw.start().await.unwrap();

    let sid = claw.new_session(None);
    let _ = claw.run_chat(&sid, "hi", None, None).await;

    let sys = system_of(&mock, 0);
    assert!(
        sys.trim_end().ends_with("just a note"),
        "empty title must render bare content at the tail: {sys}"
    );
    assert!(
        !sys.contains("## \njust a note"),
        "empty title must not emit an empty heading"
    );
    claw.stop().await.unwrap();
}

#[tokio::test]
async fn duplicate_titles_render_twice_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let (claw, mock) = build_claw(dir.path(), vec![MockReply::Text("ok".into())]);
    claw.set_injections(vec![
        sec("dup", "first body"),
        sec("dup", "second body"),
    ]);
    claw.start().await.unwrap();

    let sid = claw.new_session(None);
    let _ = claw.run_chat(&sid, "hi", None, None).await;

    let sys = system_of(&mock, 0);
    assert_eq!(sys.matches("## dup").count(), 2, "both sections rendered");
    let first = sys.find("first body").expect("first body");
    let second = sys.find("second body").expect("second body");
    assert!(first < second, "duplicate titles keep insertion order");
    claw.stop().await.unwrap();
}

#[tokio::test]
async fn multiline_and_unicode_content_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let (claw, mock) = build_claw(dir.path(), vec![MockReply::Text("ok".into())]);
    let content = "alpha\nbeta\n\n  indented 能力：浏览器 🦞 fastbrowser ✅";
    claw.set_injections(vec![sec("multi", content)]);
    claw.start().await.unwrap();

    let sid = claw.new_session(None);
    let _ = claw.run_chat(&sid, "hi", None, None).await;

    let sys = system_of(&mock, 0);
    assert!(
        sys.contains(&format!("## multi\n{content}")),
        "multiline/unicode content must be preserved: {sys}"
    );
    claw.stop().await.unwrap();
}

#[tokio::test]
async fn large_content_reaches_provider_intact() {
    let dir = tempfile::tempdir().unwrap();
    let (claw, mock) = build_claw(dir.path(), vec![MockReply::Text("ok".into())]);
    let big = "A".repeat(20_000);
    claw.set_injections(vec![sec("big", &big)]);
    claw.start().await.unwrap();

    let sid = claw.new_session(None);
    let _ = claw.run_chat(&sid, "hi", None, None).await;

    let sys = system_of(&mock, 0);
    assert!(sys.contains(&big), "large injection must not be truncated");
    claw.stop().await.unwrap();
}

// ── snapshot / storage semantics ────────────────────────────────────────────

#[test]
fn injections_snapshot_is_isolated_from_internal_state() {
    let dir = tempfile::tempdir().unwrap();
    let claw = build(dir.path());
    claw.set_injections(vec![sec("a", "1")]);

    let mut snapshot = claw.injections();
    snapshot.push(sec("b", "2"));
    snapshot[0].title = "mutated".into();

    let stored = claw.injections();
    assert_eq!(stored.len(), 1, "external mutation must not affect storage");
    assert_eq!(stored[0].title, "a");
}

#[test]
fn whitespace_only_sections_are_stored_but_skipped_in_render() {
    let dir = tempfile::tempdir().unwrap();
    let claw = build(dir.path());
    claw.set_injections(vec![sec("blank", "   \n\t"), sec("real", "ok")]);

    let stored = claw.injections();
    assert_eq!(stored.len(), 2, "storage keeps sections verbatim");
    assert_eq!(stored[0].content, "   \n\t");
}

#[test]
fn prompt_section_accepts_owned_and_borrowed_strings() {
    let owned = PromptSection::new(String::from("t"), String::from("c"));
    let borrowed = PromptSection::new("t", "c");
    assert_eq!(owned, borrowed);
    assert!(format!("{owned:?}").contains("PromptSection"));
}
