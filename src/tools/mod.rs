// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Built-in tools (`run_shell`, `run_skills`) + a session-aware tool node.

pub mod security;
pub mod shell;
pub mod skills;

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use fastmind::{Node, NodeOutput, State, Tool, ToolRegistry, Value};
use serde_json::json;

use crate::config::Settings;
use crate::error::{Error, Result};
use crate::skills::SkillRegistry;
use crate::workspace::WorkDirManager;
use crate::workspace::WorkspacePaths;

/// Shared context the built-in tools need (workdirs, settings, skills, paths).
pub struct ToolContext {
    pub paths: WorkspacePaths,
    pub workdirs: Arc<WorkDirManager>,
    settings: Arc<RwLock<Settings>>,
    pub skills: Arc<SkillRegistry>,
}

impl ToolContext {
    pub fn new(
        paths: WorkspacePaths,
        workdirs: Arc<WorkDirManager>,
        settings: Arc<RwLock<Settings>>,
        skills: Arc<SkillRegistry>,
    ) -> Arc<Self> {
        Arc::new(ToolContext {
            paths,
            workdirs,
            settings,
            skills,
        })
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn set_settings(&self, settings: Settings) {
        if let Ok(mut guard) = self.settings.write() {
            *guard = settings;
        }
    }

    /// Resolve the working directory for a session given the agent's work dirs.
    pub fn resolve_workdir(&self, session_id: &str, agent_work_dirs: &[PathBuf]) -> PathBuf {
        self.workdirs.resolve(session_id, agent_work_dirs)
    }

    /// Dispatch a tool call by name. `run_shell` / `run_skills` are handled
    /// with session context; other tools fall back to their registry `func`.
    pub async fn execute(&self, name: &str, state: &mut State, args: Value) -> Result<Value> {
        match name {
            "run_shell" => {
                let session_id = state.session_id().to_string();
                let agent_work_dirs = agent_work_dirs(state);
                let command = args
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if command.is_empty() {
                    return Ok(json!("Error: 'command' is required"));
                }
                let max_length = args.get("max_length").and_then(|v| v.as_i64());
                let timeout = args.get("timeout").and_then(|v| v.as_u64());
                let out = shell::run_shell(
                    self,
                    &session_id,
                    &agent_work_dirs,
                    state,
                    command,
                    max_length,
                    timeout,
                )
                .await?;
                Ok(json!(out))
            }
            "run_skills" => {
                let skill_name = args
                    .get("skill_name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let params = args.get("params").cloned();
                let timeout = args.get("timeout").and_then(|v| v.as_u64());
                let out = skills::run_skills(self, skill_name, params, timeout).await?;
                Ok(json!(out))
            }
            _ => Err(Error::NotFound(format!("Tool '{name}' not found"))),
        }
    }
}

fn agent_work_dirs(state: &State) -> Vec<PathBuf> {
    state
        .get("_agent_config")
        .and_then(|v| serde_json::from_value::<crate::config::AgentConfig>(v.clone()).ok())
        .map(|c| c.work_dirs)
        .unwrap_or_default()
}

/// The OpenAI function schema for `run_shell`.
pub fn shell_tool_schema() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "run_shell",
            "description": "Execute a shell command and return output. Use max_length to control output length, timeout to control execution time.",
            "parameters": {
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The shell command to execute" },
                    "max_length": { "type": "integer", "description": "Maximum output characters. Default 8000. Pass -1 for full output (optional)." },
                    "timeout": { "type": "integer", "description": "Command timeout in seconds. Default 60 (optional)." }
                },
                "required": ["command"]
            }
        }
    })
}

/// The OpenAI function schema for `run_skills`.
pub fn skills_tool_schema() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "run_skills",
            "description": "Execute a predefined skill. Use timeout to control execution time.",
            "parameters": {
                "type": "object",
                "properties": {
                    "skill_name": { "type": "string", "description": "The skill name, or '__list__' to list, or '__info__' to inspect" },
                    "params": { "type": "object", "description": "Parameters for the skill (optional)" },
                    "timeout": { "type": "integer", "description": "Skill execution timeout in seconds. Default 60 (optional)." }
                },
                "required": ["skill_name"]
            }
        }
    })
}

/// Register the built-in tools (schemas only) into a registry.
pub fn register_builtin_tools(registry: &ToolRegistry) {
    let shell = Tool::with_schema(
        "run_shell",
        "Execute a shell command and return output.",
        shell_tool_schema(),
        Arc::new(|_args, _ctx| {
            Box::pin(async move { Ok(json!("run_shell handled by session tool node")) })
        }),
    );
    let skills = Tool::with_schema(
        "run_skills",
        "Execute a predefined skill.",
        skills_tool_schema(),
        Arc::new(|_args, _ctx| {
            Box::pin(async move { Ok(json!("run_skills handled by session tool node")) })
        }),
    );
    registry.add("run_shell", shell);
    registry.add("run_skills", skills);
}

/// A session-aware tool node that emits `stream.tool_result` events.
pub struct ToolNodeWithEvents {
    pub tools: Arc<ToolRegistry>,
    pub ctx: Arc<ToolContext>,
}

impl ToolNodeWithEvents {
    pub fn new(tools: Arc<ToolRegistry>, ctx: Arc<ToolContext>) -> Self {
        ToolNodeWithEvents { tools, ctx }
    }

    fn normalize_arguments(arguments: Value) -> Value {
        match arguments {
            Value::String(s) => serde_json::from_str(&s).unwrap_or_else(|_| json!({})),
            Value::Object(_) => arguments,
            _ => json!({}),
        }
    }
}

#[async_trait]
impl Node for ToolNodeWithEvents {
    async fn run(&self, state: &mut State, event: &fastmind::Event) -> NodeOutput {
        let calls = state.tool_calls();
        if calls.is_empty() {
            return NodeOutput::none();
        }

        let session_id = state.session_id().to_string();
        let msg_id = state
            .get("_message_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                event
                    .payload
                    .get("message_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            });

        let mut results = Vec::new();
        let mut out_events = Vec::new();

        for call in calls {
            let name = call.function.name.clone();
            let arguments = Self::normalize_arguments(call.function.arguments.clone());

            let result = match self.ctx.execute(&name, state, arguments.clone()).await {
                Ok(v) => v,
                Err(_) => {
                    // Fall back to any host-registered tool with the same name.
                    match self.tools.get(&name) {
                        Some(tool) => match tool.call_simple(arguments.clone()).await {
                            Ok(v) => v,
                            Err(e) => Value::from(format!("Error executing tool: {e}")),
                        },
                        None => Value::from(format!("Tool '{name}' not found")),
                    }
                }
            };

            let result_str = crate::agent::agent::value_to_text(&result);
            out_events.push(fastmind::Event::new(
                "stream.tool_result",
                json!({
                    "tool_call_id": call.id.clone().unwrap_or_default(),
                    "tool_name": name,
                    "result": result_str,
                    "message_id": msg_id,
                }),
                session_id.clone(),
            ));

            results.push(fastmind::ToolResult {
                tool_call_id: call.id.clone(),
                tool_name: name,
                result,
            });
        }

        state.set_tool_results(results);
        state.remove("tool_calls");
        NodeOutput::Events(out_events)
    }
}
