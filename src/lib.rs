// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! # fastclaw
//!
//! An embeddable client-side AI agent library built on [`fastmind`]:
//! a ReAct loop (agent ⇄ tools) with two built-in tools (`run_shell`,
//! `run_skills`), a meta-skill subsystem, per-task working-directory scoping,
//! and an OpenAI / Anthropic dual-gateway LLM abstraction.
//!
//! It is designed for embedding into desktop (Tauri/TUI) and mobile
//! (iOS/Android/HarmonyOS) hosts — no HTTP/WebSocket server of its own.
//!
//! ```no_run
//! use fastclaw::{FastClaw, FastClawConfig};
//!
//! #[tokio::main]
//! async fn main() -> fastclaw::Result<()> {
//!     let mut claw = FastClaw::builder(FastClawConfig::default()).build();
//!     claw.start().await?;
//!     claw.chat("session-1", "Hello").await?;
//!     claw.stop().await?;
//!     Ok(())
//! }
//! ```

#![forbid(unsafe_code)]
#![allow(clippy::type_complexity)]

pub mod agent;
pub mod api;
pub mod config;
pub mod error;
pub mod llm;
#[cfg(feature = "tui")]
pub mod markdown;
pub mod seed;
pub mod session;
pub mod skills;
pub mod tools;
pub mod workspace;

pub use api::{FastClaw, FastClawBuilder, FastClawConfig, StreamEvent};
pub use config::{AgentConfig, Settings};
pub use error::{Error, ErrorCode, Result};
pub use llm::{
    ImagePart, LlmGateway, LlmProvider, LlmRequest, LlmResponse, LlmSink, Message, ThinkingPref,
    ToolCall, ToolSchema,
};
pub use skills::{Skill, SkillInfo, SkillKind, SkillRegistry};
pub use workspace::{WorkDirManager, WorkspacePaths};

// Re-export fastmind so host code can use a single dependency.
pub use fastmind;
pub use fastmind::serde_json;
pub use fastmind::{Event, EventType, State, Value};

/// Version string.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod prelude {
    pub use crate::{
        FastClaw, FastClawConfig, LlmGateway, Result, Skill, SkillInfo, WorkDirManager,
        WorkspacePaths,
    };
    pub use fastmind::serde_json::json;
}
