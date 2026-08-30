// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Shared test helpers (mock LLM provider, context builders).

#![allow(dead_code)]

use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

use async_trait::async_trait;
use fastclaw::llm::provider::NullSink;
use fastclaw::tools::ToolContext;
use fastclaw::{
    LlmProvider, LlmRequest, LlmResponse, LlmSink, Settings, SkillRegistry, ToolCall,
    WorkDirManager, WorkspacePaths,
};

/// A scripted reply from the mock provider.
#[derive(Clone)]
pub enum MockReply {
    Text(String),
    /// Emit reasoning deltas then text deltas (for `stream.thinking` tests),
    /// optionally carrying an Anthropic thinking signature.
    TextThinking {
        reasoning: String,
        text: String,
        signature: Option<String>,
    },
    /// A list of (tool_name, arguments_json) tool calls.
    Tool(Vec<(String, String)>),
}

/// A scripted LLM provider that replays a queue of replies.
pub struct MockProvider {
    pub script: Mutex<Vec<MockReply>>,
    pub calls: Mutex<Vec<LlmRequest>>,
}

impl MockProvider {
    pub fn new(script: Vec<MockReply>) -> Arc<Self> {
        Arc::new(MockProvider {
            script: Mutex::new(script),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

#[async_trait]
impl LlmProvider for MockProvider {
    async fn chat(&self, req: &LlmRequest) -> fastclaw::Result<LlmResponse> {
        self.chat_stream(req, &mut NullSink).await
    }

    async fn chat_stream(
        &self,
        req: &LlmRequest,
        sink: &mut dyn LlmSink,
    ) -> fastclaw::Result<LlmResponse> {
        self.calls.lock().unwrap().push(req.clone());
        let reply = self.script.lock().unwrap().remove(0);
        Ok(match reply {
            MockReply::Text(t) => {
                for ch in t.chars() {
                    sink.on_text(&ch.to_string());
                }
                LlmResponse {
                    text: t,
                    ..Default::default()
                }
            }
            MockReply::TextThinking {
                reasoning,
                text,
                signature,
            } => {
                for ch in reasoning.chars() {
                    sink.on_thinking(&ch.to_string());
                }
                for ch in text.chars() {
                    sink.on_text(&ch.to_string());
                }
                LlmResponse {
                    text,
                    reasoning: Some(reasoning),
                    thinking_signature: signature,
                    ..Default::default()
                }
            }
            MockReply::Tool(args) => {
                sink.on_tool_call_start();
                let calls = args
                    .into_iter()
                    .enumerate()
                    .map(|(i, (n, a))| ToolCall {
                        id: format!("call_{i}"),
                        name: n,
                        arguments: a,
                    })
                    .collect();
                LlmResponse {
                    text: String::new(),
                    tool_calls: calls,
                    ..Default::default()
                }
            }
        })
    }
}

/// Build a `ToolContext` rooted at `root`.
pub fn make_tool_context(root: &Path) -> Arc<ToolContext> {
    let paths = WorkspacePaths::new(root.to_path_buf());
    let _ = paths.ensure();
    let workdirs = Arc::new(WorkDirManager::new(root.join("work")));
    let settings = Arc::new(RwLock::new(Settings::default()));
    let skills = SkillRegistry::new(paths.clone());
    ToolContext::new(paths, workdirs, settings, skills)
}

/// Build a shared `SkillRegistry` rooted at `root` (for skills tests).
pub fn make_skills(root: &Path) -> Arc<SkillRegistry> {
    let paths = WorkspacePaths::new(root.to_path_buf());
    let _ = paths.ensure();
    SkillRegistry::new(paths)
}

/// Push a chat message and collect every emitted event until a terminal one.
pub async fn collect_events(
    claw: &fastclaw::FastClaw,
    session: &str,
    text: &str,
) -> Vec<fastclaw::Event> {
    let mut events = Vec::new();
    claw.chat(session, text).await.unwrap();
    if let Ok(mut stream) = claw.stream_events(session) {
        use futures::StreamExt;
        while let Some(ev) = stream.next().await {
            let terminal = matches!(ev.type_.as_str(), "stream.end" | "stream.error");
            events.push(ev);
            if terminal {
                break;
            }
        }
    }
    events
}

/// Build a `FastClaw` rooted at `root` with a scripted mock provider injected.
pub fn build_claw(root: &Path, script: Vec<MockReply>) -> (fastclaw::FastClaw, Arc<MockProvider>) {
    let claw = fastclaw::FastClaw::builder(fastclaw::FastClawConfig {
        workspace_root: Some(root.to_path_buf()),
        ..Default::default()
    })
    .build();
    let mock = MockProvider::new(script);
    claw.set_mock_provider(mock.clone());
    (claw, mock)
}
