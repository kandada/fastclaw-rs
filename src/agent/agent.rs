// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! The ReAct agent: a streaming LLM node + routing + graph assembly.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use fastmind::{Event, EventBuffer, Graph, Node, NodeOutput, State, ToolRegistry, Value};
use serde_json::json;

use crate::agent::system_prompt::format_system_prompt;
use crate::config::{
    load_agent_config, load_agent_personality, resolve_session_agent_id, AgentConfig, Settings,
};
use crate::llm::provider::Message;
use crate::llm::{LlmProvider, LlmRequest, LlmSink, ToolSchema};
use crate::session::{
    fix_invalid_tool_calls, load_messages_from_jsonl, save_messages_to_jsonl, unload_early_messages,
};
use crate::skills::SkillRegistry;
use crate::tools::ToolNodeWithEvents;
use crate::workspace::{SessionStore, WorkDirManager, WorkspacePaths};

/// Shared runtime the agent node and tool node depend on.
pub struct AgentRuntime {
    pub paths: WorkspacePaths,
    pub workdirs: Arc<WorkDirManager>,
    pub settings: Arc<RwLock<Settings>>,
    pub session_store: SessionStore,
    pub skills: Arc<SkillRegistry>,
    pub tool_registry: Arc<ToolRegistry>,
    providers: RwLock<HashMap<String, Arc<dyn LlmProvider>>>,
    mock: RwLock<Option<Arc<dyn LlmProvider>>>,
}

impl AgentRuntime {
    pub fn new(
        paths: WorkspacePaths,
        workdirs: Arc<WorkDirManager>,
        settings: Arc<RwLock<Settings>>,
        skills: Arc<SkillRegistry>,
        tool_registry: Arc<ToolRegistry>,
    ) -> Arc<Self> {
        let session_store = SessionStore::new(paths.sessions_db());
        Arc::new(AgentRuntime {
            paths,
            workdirs,
            settings,
            session_store,
            skills,
            tool_registry,
            providers: RwLock::new(HashMap::new()),
            mock: RwLock::new(None),
        })
    }

    /// Inject a fallback provider (used by tests and when no API key is set).
    pub fn set_mock_provider(&self, provider: Arc<dyn LlmProvider>) {
        if let Ok(mut g) = self.mock.write() {
            *g = Some(provider);
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn set_settings(&self, settings: Settings) {
        if let Ok(mut g) = self.settings.write() {
            *g = settings;
        }
    }

    /// Get (or lazily build + cache) the LLM provider for an agent config.
    pub fn get_provider(&self, agent_config: &AgentConfig) -> Option<Arc<dyn LlmProvider>> {
        let llm = &agent_config.llm;
        let key = format!(
            "{}:{}:{}:{}",
            llm.gateway, llm.api_key, llm.base_url, llm.model
        );
        if let Some(p) = self
            .providers
            .read()
            .ok()
            .and_then(|g| g.get(&key).cloned())
        {
            return Some(p);
        }
        if let Some(provider) = crate::llm::build_provider(
            llm.gateway,
            llm.model.clone(),
            llm.api_key.clone(),
            llm.base_url.clone(),
        ) {
            if let Ok(mut g) = self.providers.write() {
                g.insert(key, provider.clone());
            }
            return Some(provider);
        }
        // Fall back to an injected mock (tests / no-key hosts).
        self.mock.read().ok().and_then(|g| g.clone())
    }
}

/// Convert a fastmind OpenAI tool-schema value into a normalized `ToolSchema`.
fn to_tool_schema(schema: &Value) -> Option<ToolSchema> {
    let f = schema.get("function")?;
    let name = f.get("name")?.as_str()?.to_string();
    let description = f
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let parameters = f
        .get("parameters")
        .cloned()
        .unwrap_or_else(|| json!({ "type": "object", "properties": {}, "required": [] }));
    Some(ToolSchema {
        name,
        description,
        parameters,
    })
}

fn emit(buf: &Option<Arc<EventBuffer>>, type_: &str, payload: Value, session_id: &str) {
    if let Some(buf) = buf {
        buf.append(Event::new(type_, payload, session_id));
    }
}

/// A string value renders as its raw text; other JSON values as compact JSON.
pub fn value_to_text(value: &Value) -> String {
    value
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| value.to_string())
}

fn get_messages(state: &State) -> Vec<Value> {
    state
        .get("messages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
}

fn set_messages(state: &mut State, messages: Vec<Value>) {
    state.set("messages", Value::Array(messages));
}

/// Bind the session's agent config / personality / history into state (lazily).
fn ensure_agent_bound(runtime: &AgentRuntime, state: &mut State, session_id: &str) {
    let current = resolve_session_agent_id(&runtime.paths, session_id);
    let bound = state
        .get("_bound_agent_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !state.contains_key("_agent_config") || (!current.is_empty() && current != bound) {
        let config = load_agent_config(&runtime.paths, &current);
        let personality = load_agent_personality(&runtime.paths, &current);
        let existing = load_messages_from_jsonl(&runtime.paths.sessions_dir(), session_id);
        state.set(
            "_agent_config",
            serde_json::to_value(&config).unwrap_or_else(|_| json!({})),
        );
        state.set("_bound_agent_id", json!(current));
        state.set("_personality", json!(personality));
        if state.get("messages").is_none() {
            state.set("messages", Value::Array(existing));
        }
    }
}

fn maybe_name_session(runtime: &AgentRuntime, session_id: &str, messages: &[Value]) {
    let first = match messages
        .iter()
        .find(|m| m.get("role").and_then(|v| v.as_str()) == Some("user"))
        .and_then(|m| m.get("content").and_then(|v| v.as_str()))
    {
        Some(c) => c.chars().take(50).collect::<String>(),
        None => return,
    };
    let mut sessions = runtime.session_store.load();
    if let Some(s) = sessions.get_mut(session_id) {
        let has_name = s
            .get("name")
            .and_then(|v| v.as_str())
            .map(|n| !n.is_empty())
            .unwrap_or(false);
        if !has_name {
            s["name"] = json!(first);
            runtime.session_store.save(&sessions);
        }
    }
}

/// Streaming sink that forwards reasoning/content deltas as events.
struct AgentSink {
    buf: Option<Arc<EventBuffer>>,
    session_id: String,
    msg_id: Option<String>,
    full_content: String,
    has_tool_calls: bool,
}

impl LlmSink for AgentSink {
    fn on_thinking(&mut self, delta: &str) {
        emit(
            &self.buf,
            "stream.thinking",
            json!({ "delta": delta, "message_id": self.msg_id }),
            &self.session_id,
        );
    }

    fn on_text(&mut self, delta: &str) {
        if !self.has_tool_calls {
            self.full_content.push_str(delta);
            emit(
                &self.buf,
                "stream.chunk",
                json!({ "delta": delta, "message_id": self.msg_id }),
                &self.session_id,
            );
        }
    }

    fn on_tool_call_start(&mut self) {
        self.has_tool_calls = true;
    }
}

/// The agent node's core logic. Mutates `state` in place and streams events.
async fn agent_loop(runtime: &AgentRuntime, state: &mut State, event: &Event) -> NodeOutput {
    let session_id = state.session_id().to_string();
    let buf = state.output().cloned();

    ensure_agent_bound(runtime, state, &session_id);

    let user_text = event
        .payload
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let images: Option<Vec<Value>> = event
        .payload
        .get("images")
        .and_then(|v| v.as_array())
        .cloned();
    let msg_id = event
        .payload
        .get("message_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    if let Some(id) = &msg_id {
        state.set("_message_id", json!(id));
    }

    // Process tool results into `role: "tool"` messages.
    let tool_results = state.tool_results();
    if !tool_results.is_empty() {
        let mut messages = get_messages(state);
        let insert_at = messages
            .iter()
            .rposition(|m| {
                m.get("role").and_then(|v| v.as_str()) == Some("assistant")
                    && m.get("tool_calls")
                        .and_then(|v| v.as_array())
                        .map(|a| !a.is_empty())
                        .unwrap_or(false)
            })
            .map(|i| i + 1);
        let tool_msgs: Vec<Value> = tool_results
            .iter()
            .map(|r| {
                json!({
                    "role": "tool",
                    "tool_call_id": r.tool_call_id.clone().unwrap_or_default(),
                    "content": value_to_text(&r.result),
                })
            })
            .collect();
        if let Some(idx) = insert_at {
            for (k, tm) in tool_msgs.into_iter().enumerate() {
                messages.insert(idx + k, tm);
            }
        } else {
            tracing::warn!(
                "tool_results orphaned: no matching assistant(tool_calls) (session={})",
                session_id
            );
        }
        set_messages(state, messages);
    }
    state.remove("tool_results");

    // Append the user message (dedup by message_id, fallback content).
    let mut messages = get_messages(state);
    let is_new = match &msg_id {
        Some(id) => state.get("_last_user_msg_id").and_then(|v| v.as_str()) != Some(id.as_str()),
        None => !messages.iter().any(|m| {
            m.get("role").and_then(|v| v.as_str()) == Some("user")
                && m.get("content").and_then(|v| v.as_str()) == Some(user_text.as_str())
        }),
    };
    if is_new {
        if let Some(id) = &msg_id {
            state.set("_last_user_msg_id", json!(id));
        }
        let mut user_msg = json!({ "role": "user", "content": user_text });
        if let Some(imgs) = &images {
            user_msg["images"] = Value::Array(imgs.clone());
        }
        messages.push(user_msg);
        set_messages(state, messages.clone());
        save_messages_to_jsonl(&runtime.paths.sessions_dir(), &session_id, &messages);
        maybe_name_session(runtime, &session_id, &messages);
    }

    let agent_config: AgentConfig = state
        .get("_agent_config")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_else(|| AgentConfig::default_for("main_agent"));

    // Context management.
    let threshold = agent_config.context.unload_threshold_tokens.max(1) as usize;
    let (llm_messages, _unloaded) = unload_early_messages(&messages, threshold);
    let llm_messages = fix_invalid_tool_calls(&llm_messages);

    let provider = match runtime.get_provider(&agent_config) {
        Some(p) => p,
        None => {
            emit(
                &buf,
                "stream.error",
                json!({ "error": "No LLM provider configured (missing API key)", "message_id": msg_id }),
                &session_id,
            );
            return NodeOutput::none();
        }
    };

    // System prompt.
    let skills_list = runtime.skills.skills_list_string();
    let personality = state
        .get("_personality")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let work_dirs: Vec<String> = agent_config
        .work_dirs
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let workspace_path = runtime.paths.root.to_string_lossy().into_owned();
    let workdir = runtime
        .workdirs
        .resolve(&session_id, &agent_config.work_dirs);
    let system = format_system_prompt(
        &skills_list,
        &session_id,
        &personality,
        &work_dirs,
        &workspace_path,
        &workdir.to_string_lossy(),
    );

    // Build request.
    let messages_norm: Vec<Message> = llm_messages.iter().map(Message::from_value).collect();
    let tools: Vec<ToolSchema> = runtime
        .tool_registry
        .get_schemas()
        .iter()
        .filter_map(to_tool_schema)
        .collect();
    let req = LlmRequest {
        system: Some(system),
        messages: messages_norm,
        tools,
        model: agent_config.llm.model.clone(),
        max_tokens: agent_config.llm.max_tokens,
        thinking: match agent_config.llm.enable_thinking {
            Some(false) => crate::llm::ThinkingPref::Disabled,
            _ => match agent_config.llm.thinking_budget_tokens {
                Some(b) if b > 0 => crate::llm::ThinkingPref::Enabled(b),
                _ => crate::llm::ThinkingPref::Adaptive,
            },
        },
    };

    // Stream.
    let mut sink = AgentSink {
        buf: buf.clone(),
        session_id: session_id.clone(),
        msg_id: msg_id.clone(),
        full_content: String::new(),
        has_tool_calls: false,
    };
    let resp = provider.chat_stream(&req, &mut sink).await;

    let (full_content, reasoning, tool_calls, thinking_signature) = match resp {
        Ok(r) => (
            sink.full_content,
            r.reasoning,
            r.tool_calls,
            r.thinking_signature,
        ),
        Err(e) => {
            emit(
                &buf,
                "stream.error",
                json!({ "error": e.to_string(), "message_id": msg_id }),
                &session_id,
            );
            return NodeOutput::none();
        }
    };

    let tcs_value: Vec<Value> = tool_calls
        .iter()
        .map(|tc| {
            json!({
                "id": tc.id,
                "type": "function",
                "function": { "name": tc.name, "arguments": tc.arguments },
            })
        })
        .collect();

    // Persist the assistant message.
    let mut messages = get_messages(state);
    let mut amsg = if tool_calls.is_empty() {
        json!({ "role": "assistant", "content": full_content.clone() })
    } else {
        json!({ "role": "assistant", "content": full_content.clone(), "tool_calls": tcs_value.clone() })
    };
    if let Some(r) = &reasoning {
        amsg["reasoning_content"] = json!(r);
    }
    if let Some(sig) = &thinking_signature {
        amsg["thinking_signature"] = json!(sig);
    }
    messages.push(amsg);
    set_messages(state, messages.clone());
    save_messages_to_jsonl(&runtime.paths.sessions_dir(), &session_id, &messages);

    if !tool_calls.is_empty() {
        emit(
            &buf,
            "stream.fragment",
            json!({
                "content": full_content,
                "has_tool_calls": true,
                "tool_calls": tcs_value,
                "message_id": msg_id,
            }),
            &session_id,
        );

        let calls: Vec<fastmind::ToolCall> = tool_calls
            .iter()
            .map(|tc| fastmind::ToolCall {
                id: Some(tc.id.clone()),
                type_: Some("function".to_string()),
                function: fastmind::FunctionCall {
                    name: tc.name.clone(),
                    arguments: Value::String(tc.arguments.clone()),
                },
            })
            .collect();
        state.set_tool_calls(calls);
    } else {
        emit(
            &buf,
            "stream.end",
            json!({ "message_id": msg_id }),
            &session_id,
        );
    }

    NodeOutput::none()
}

/// Build the agent node from the runtime.
pub fn build_node(runtime: Arc<AgentRuntime>) -> Arc<dyn Node> {
    fastmind::fn_node(move |state: &mut State, event: &Event| {
        let runtime = runtime.clone();
        Box::pin(async move { agent_loop(&runtime, state, event).await })
    })
}

/// Router: `tool_calls` present → `tools`, else end.
pub fn route(state: &State, _event: &Event) -> Option<String> {
    if state.contains_key("tool_calls") {
        Some("tools".to_string())
    } else {
        None
    }
}

/// Assemble the ReAct graph.
pub fn build_graph(runtime: Arc<AgentRuntime>, ctx: Arc<crate::tools::ToolContext>) -> Graph {
    let mut graph = Graph::new();
    graph.add_node("agent", build_node(runtime.clone()));
    graph.add_node(
        "tools",
        ToolNodeWithEvents::new(runtime.tool_registry.clone(), ctx),
    );
    graph.add_conditional_edges(
        "agent",
        Box::new(route),
        HashMap::from([
            (Some("tools".to_string()), "tools".to_string()),
            (None, "__end__".to_string()),
        ]),
        vec![],
    );
    graph.add_edge("tools", "agent", None);
    graph.set_entry_point("agent");
    graph
}
