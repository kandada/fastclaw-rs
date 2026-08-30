// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Performance & resource regression tests for fastclaw-rs.
//!
//! These exercise the real ReAct chat pipeline (mock LLM), session churn and
//! long conversations, and measure throughput, CPU utilization and resident
//! memory growth. They are `#[ignore]`d by default; run them with:
//!
//! ```text
//! cargo test --release --test perf -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Assertions are deliberately loose — they catch gross regressions and
//! unbounded memory growth; the printed `PERF[...]` lines are the tracking log.

use std::time::{Duration, Instant};

mod common;
use common::{build_claw, MockReply};

// ── process sampling helpers (macOS + Linux `ps`) ───────────────────────────

fn ps_field(format: &str) -> String {
    std::process::Command::new("ps")
        .args(["-o", format, "-p", &std::process::id().to_string()])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn current_rss_kb() -> u64 {
    ps_field("rss=").parse().unwrap_or(0)
}

fn min_rss_kb(samples: usize) -> u64 {
    (0..samples).map(|_| current_rss_kb()).min().unwrap_or(0)
}

/// (user, system) CPU seconds consumed by this process so far.
fn cpu_times_secs() -> (f64, f64) {
    let out = std::process::Command::new("ps")
        .args([
            "-o",
            "utime=",
            "-o",
            "stime=",
            "-p",
            &std::process::id().to_string(),
        ])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let mut it = out.split_whitespace();
    (parse_ps_time(it.next()), parse_ps_time(it.next()))
}

fn parse_ps_time(t: Option<&str>) -> f64 {
    let s = t.unwrap_or("0").trim();
    if let Some((min, sec)) = s.split_once(':') {
        let m: f64 = min.parse().unwrap_or(0.0);
        let ss: f64 = sec.parse().unwrap_or(0.0);
        m * 60.0 + ss
    } else {
        s.parse().unwrap_or(0.0)
    }
}

fn report(name: &str, ops: u64, wall: Duration, cpu_user: f64, cpu_sys: f64) {
    let wall_s = wall.as_secs_f64().max(1e-9);
    let cpu_s = cpu_user + cpu_sys;
    println!(
        "PERF[{name}] ops={ops} wall={wall_s:.3}s cpu={cpu_s:.3}s util={:.1}% throughput={:.0} ops/s",
        (cpu_s / wall_s) * 100.0,
        ops as f64 / wall_s
    );
}

fn script_of_texts(n: usize) -> Vec<MockReply> {
    (0..n)
        .map(|i| MockReply::Text(format!("reply {i}")))
        .collect()
}

// ── throughput / latency ────────────────────────────────────────────────────

#[ignore = "perf: run explicitly"]
#[tokio::test]
async fn perf_chat_pipeline_text_turns() {
    const N: usize = 200;
    let dir = tempfile::tempdir().unwrap();
    let (claw, _mock) = build_claw(dir.path(), script_of_texts(N));
    claw.start().await.unwrap();
    let sid = claw.new_session(None);

    let (u0, s0) = cpu_times_secs();
    let t = Instant::now();
    for i in 0..N {
        let reply = claw.run_chat(&sid, &format!("msg {i}"), None, None).await;
        assert_eq!(reply, format!("reply {i}"), "turn {i} reply mismatch");
    }
    let wall = t.elapsed();
    let (u1, s1) = cpu_times_secs();
    report(
        "chat/text_turns_same_session",
        N as u64,
        wall,
        u1 - u0,
        s1 - s0,
    );
    claw.stop().await.unwrap();
}

#[ignore = "perf: run explicitly"]
#[tokio::test]
async fn perf_chat_pipeline_session_churn() {
    const N: usize = 200;
    let dir = tempfile::tempdir().unwrap();
    let (claw, _mock) = build_claw(dir.path(), script_of_texts(N));
    claw.start().await.unwrap();

    let (u0, s0) = cpu_times_secs();
    let t = Instant::now();
    for i in 0..N {
        let sid = claw.new_session(None);
        let reply = claw.run_chat(&sid, &format!("msg {i}"), None, None).await;
        assert_eq!(reply, format!("reply {i}"), "turn {i} reply mismatch");
        claw.delete_session(&sid).await;
    }
    let wall = t.elapsed();
    let (u1, s1) = cpu_times_secs();
    report(
        "chat/session_create_chat_delete",
        N as u64,
        wall,
        u1 - u0,
        s1 - s0,
    );
    claw.stop().await.unwrap();
}

#[ignore = "perf: run explicitly"]
#[tokio::test]
async fn perf_react_loop_with_tool_roundtrip() {
    const N: usize = 100;
    let dir = tempfile::tempdir().unwrap();
    let mut script = Vec::new();
    for i in 0..N {
        script.push(MockReply::Tool(vec![(
            "run_shell".into(),
            "{\"command\":\"echo perf\"}".into(),
        )]));
        script.push(MockReply::Text(format!("done {i}")));
    }
    let (claw, mock) = build_claw(dir.path(), script);
    claw.start().await.unwrap();
    let sid = claw.new_session(None);

    let (u0, s0) = cpu_times_secs();
    let t = Instant::now();
    for i in 0..N {
        let reply = claw.run_chat(&sid, &format!("task {i}"), None, None).await;
        assert_eq!(reply, format!("done {i}"), "turn {i} reply mismatch");
    }
    let wall = t.elapsed();
    let (u1, s1) = cpu_times_secs();
    assert_eq!(mock.call_count(), 2 * N);
    report(
        "chat/react_agent_tool_roundtrip",
        N as u64,
        wall,
        u1 - u0,
        s1 - s0,
    );
    claw.stop().await.unwrap();
}

// ── CPU utilization ─────────────────────────────────────────────────────────

#[ignore = "perf: run explicitly"]
#[tokio::test]
async fn cpu_utilization_chat_load() {
    const BATCH: usize = 20;
    let dir = tempfile::tempdir().unwrap();
    let (claw, _mock) = build_claw(dir.path(), script_of_texts(1_000_000));
    claw.start().await.unwrap();
    let sid = claw.new_session(None);

    let (u0, s0) = cpu_times_secs();
    let t = Instant::now();
    let mut turns = 0u64;
    while t.elapsed() < Duration::from_secs(1) {
        for _ in 0..BATCH {
            let _ = claw.run_chat(&sid, "hi", None, None).await;
        }
        turns += BATCH as u64;
    }
    let wall = t.elapsed();
    let (u1, s1) = cpu_times_secs();
    let cpu = (u1 - u0) + (s1 - s0);
    let util = cpu / wall.as_secs_f64().max(1e-9);
    println!(
        "PERF[cpu/chat] turns={turns} wall={:.3}s cpu={:.3}s util={:.1}% throughput={:.0} turns/s",
        wall.as_secs_f64(),
        cpu,
        util * 100.0,
        turns as f64 / wall.as_secs_f64().max(1e-9)
    );
    // The pipeline is CPU+IO mixed (JSONL persistence), but must be well above
    // idle (>=20%) — far below that means the hot path is sleeping.
    assert!(
        util >= 0.20,
        "chat pipeline should burn meaningful CPU, observed util={:.1}%",
        util * 100.0
    );
    claw.stop().await.unwrap();
}

// ── memory-leak detection (RSS plateau) ─────────────────────────────────────

#[ignore = "perf: run explicitly"]
#[tokio::test]
async fn memory_leak_session_churn() {
    let dir = tempfile::tempdir().unwrap();

    // Warm up (allocator arenas, runtime threads, workspace creation).
    let (warm, _) = build_claw(dir.path(), script_of_texts(20));
    warm.start().await.unwrap();
    for _i in 0..20 {
        let sid = warm.new_session(None);
        let _ = warm.run_chat(&sid, "warm", None, None).await;
        warm.delete_session(&sid).await;
    }
    warm.stop().await.unwrap();
    drop(warm);
    let base = min_rss_kb(5);

    // Batches of increasing size; sessions are always deleted after the turn.
    let (b1, _) = build_claw(dir.path(), script_of_texts(150));
    b1.start().await.unwrap();
    for i in 0..150 {
        let sid = b1.new_session(None);
        let _ = b1.run_chat(&sid, &format!("m{i}"), None, None).await;
        b1.delete_session(&sid).await;
    }
    b1.stop().await.unwrap();
    drop(b1);
    let r1 = min_rss_kb(5);

    let (b2, _) = build_claw(dir.path(), script_of_texts(400));
    b2.start().await.unwrap();
    for i in 0..400 {
        let sid = b2.new_session(None);
        let _ = b2.run_chat(&sid, &format!("m{i}"), None, None).await;
        b2.delete_session(&sid).await;
    }
    b2.stop().await.unwrap();
    drop(b2);
    let r2 = min_rss_kb(5);

    let slope = (r2 as i64 - r1 as i64) / 400;
    println!(
        "PERF[mem/session_churn] base={}KB b1(150)={}KB b2(400)={}KB slope={}KB/op",
        base, r1, r2, slope
    );
    // After warm-up, retained memory must plateau: 400 churns growing by more
    // than 32MB (or 32KB/op) indicates a leak (e.g. sessions not released).
    assert!(
        r2 <= r1 + 32 * 1024,
        "RSS grew {} KB over 400 session churns — possible leak",
        r2.saturating_sub(r1)
    );
    assert!(slope < 32, "RSS grew {slope} KB/op — possible leak");
}

#[ignore = "perf: run explicitly"]
#[tokio::test]
async fn memory_leak_long_conversation() {
    let dir = tempfile::tempdir().unwrap();
    let (claw, _mock) = build_claw(dir.path(), script_of_texts(1_000_000));
    claw.start().await.unwrap();
    let sid = claw.new_session(None);

    for _i in 0..50 {
        let _ = claw.run_chat(&sid, "warm", None, None).await;
    }
    let base = min_rss_kb(5);

    for i in 0..100 {
        let _ = claw.run_chat(&sid, &format!("c1_{i}"), None, None).await;
    }
    let r1 = min_rss_kb(5);
    for i in 0..200 {
        let _ = claw.run_chat(&sid, &format!("c2_{i}"), None, None).await;
    }
    let r2 = min_rss_kb(5);

    let slope = (r2 as i64 - r1 as i64) / 200;
    let msgs = claw.get_messages(&sid);
    println!(
        "PERF[mem/long_conversation] base={}KB b1(100t)={}KB b2(200t)={}KB slope={}KB/turn total_msgs={}",
        base,
        r1,
        r2,
        slope,
        msgs.len()
    );
    // Conversation history is intentionally retained (state.messages + JSONL),
    // so allow modest per-turn growth, but flag runaway growth (>64KB/turn).
    assert!(
        slope < 64,
        "long conversation grew {slope} KB/turn — possible leak"
    );
    claw.stop().await.unwrap();
}
