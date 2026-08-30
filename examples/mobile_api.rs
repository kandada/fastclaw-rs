// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! FFI-boundary style usage: demonstrates the shape of an API suitable for
//! mobile/desktop hosts (opaque handle + JSON strings + callbacks, no generics).

use fastclaw::{FastClaw, FastClawConfig};

/// This is how a host (Tauri / iOS / Android / HarmonyOS) would drive the agent.
#[tokio::main]
async fn main() -> fastclaw::Result<()> {
    // On mobile, pass the app sandbox directory explicitly.
    let sandbox = std::env::temp_dir().join("fastclaw-mobile-demo");
    let claw = FastClaw::builder(FastClawConfig {
        workspace_root: Some(sandbox.clone()),
        ..FastClawConfig::default()
    })
    .build();
    claw.start().await?;

    // FFI-friendly: session ids and messages are plain strings.
    let session_id = claw.new_session(None);

    // JSON-string based payload construction (host language encodes its own JSON).
    let _ = claw.chat(&session_id, "hello").await;

    // Stream events into a flattened, JSON-payload struct.
    let mut stream = claw.stream_events(&session_id)?;
    use futures::StreamExt;
    while let Some(ev) = stream.next().await {
        let flat = fastclaw::api::StreamEvent::from_event(&ev);
        println!("event {} -> {}", flat.type_, flat.payload);
        if flat.type_ == "stream.end" || flat.type_ == "stream.error" {
            break;
        }
    }

    claw.stop().await?;
    let _ = std::fs::remove_dir_all(sandbox);
    Ok(())
}
