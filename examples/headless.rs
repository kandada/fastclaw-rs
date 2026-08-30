// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Minimal headless usage: build a `FastClaw`, start it, run a chat.
//!
//! Requires `LLM_API_KEY` (OpenAI-compatible) or `ANTHROPIC_API_KEY` to actually
//! call a model; without a key it still demonstrates the full setup + lifecycle.

use fastclaw::{FastClaw, FastClawConfig};

#[tokio::main]
async fn main() -> fastclaw::Result<()> {
    // Dev builds default to `<repo>/workspace`; release builds to `~/.fastclaw-rs`.
    let claw = FastClaw::builder(FastClawConfig::default()).build();
    claw.start().await?;

    let session_id = claw.new_session(None);
    println!("session: {session_id}");

    // Register the current directory as a working directory.
    let _ = claw.register_work_dir(std::path::Path::new("."));
    println!("work dirs: {:?}", claw.list_work_dirs());
    println!(
        "skills: {:?}",
        claw.list_skills()
            .iter()
            .map(|s| s.name.clone())
            .collect::<Vec<_>>()
    );

    // Chat (streams to stdout when a model is configured).
    let reply = claw
        .run_chat(
            &session_id,
            "Hello, please introduce yourself in one sentence.",
            Some(std::sync::Arc::new(|delta: String| print!("{delta}"))),
            Some(std::sync::Arc::new(|full: String| {
                println!("\n[done {} chars]", full.len())
            })),
        )
        .await;
    if reply.is_empty() {
        println!("(no LLM configured — set LLM_API_KEY to get a real reply)");
    }

    claw.stop().await?;
    Ok(())
}
