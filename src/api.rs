// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! The `FastClaw` facade — the single entry point for host applications
//! (TUI / Tauri / iOS / Android / HarmonyOS).

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::agent::{build_graph, AgentRuntime};
use crate::config::{self, load_settings, save_settings, AgentConfig, Settings};
use crate::error::{Error, Result};
use crate::llm::LlmGateway;
use crate::skills::{Skill, SkillRegistry};
use crate::tools::{register_builtin_tools, ToolContext};
use crate::workspace::{SessionStore, WorkDirManager, WorkspacePaths};

/// Configuration for building a `FastClaw` instance.
#[derive(Clone, Default)]
pub struct FastClawConfig {
    /// Workspace root. On mobile/embedded hosts this must be set to the app
    /// sandbox directory. When `None`, the documented resolution chain applies.
    pub workspace_root: Option<PathBuf>,
    /// Initial working directories to register.
    pub work_dirs: Vec<PathBuf>,
    /// Default gateway for agents that don't specify one.
    pub default_gateway: LlmGateway,
    /// Host-injected tools (device capabilities etc.), registered alongside the
    /// built-in `run_shell` / `run_skills`.
    pub extra_tools: Vec<Arc<fastmind::Tool>>,
    /// Override the default agent id.
    pub default_agent_id: Option<String>,
}

/// Builder for [`FastClaw`].
pub struct FastClawBuilder {
    config: FastClawConfig,
}

impl FastClawBuilder {
    pub fn new(config: FastClawConfig) -> Self {
        FastClawBuilder { config }
    }

    pub fn workspace_root(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.workspace_root = Some(path.into());
        self
    }

    pub fn work_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.work_dirs.push(path.into());
        self
    }

    pub fn tool(mut self, tool: Arc<fastmind::Tool>) -> Self {
        self.config.extra_tools.push(tool);
        self
    }

    pub fn build(self) -> FastClaw {
        FastClaw::build(self.config)
    }
}

/// A flattened stream event (FFI-friendly: payload is a JSON string).
#[derive(Debug, Clone)]
pub struct StreamEvent {
    pub type_: String,
    pub payload: String,
    pub session_id: String,
}

impl StreamEvent {
    pub fn from_event(e: &fastmind::Event) -> Self {
        StreamEvent {
            type_: e.type_.clone(),
            payload: e.payload.to_string(),
            session_id: e.session_id.clone(),
        }
    }
}

/// The FastClaw agent engine facade.
pub struct FastClaw {
    api: fastmind::FastMindAPI,
    paths: WorkspacePaths,
    workdirs: Arc<WorkDirManager>,
    settings: Arc<RwLock<Settings>>,
    session_store: SessionStore,
    skills: Arc<SkillRegistry>,
    tool_registry: Arc<fastmind::ToolRegistry>,
    ctx: Arc<ToolContext>,
    runtime: Arc<AgentRuntime>,
}

impl FastClaw {
    pub fn builder(config: FastClawConfig) -> FastClawBuilder {
        FastClawBuilder::new(config)
    }

    pub fn default_config() -> FastClawConfig {
        FastClawConfig::default()
    }

    fn build(config: FastClawConfig) -> FastClaw {
        let paths = WorkspacePaths::new(WorkspacePaths::resolve(config.workspace_root.clone()));
        let _ = paths.ensure();
        // Seed built-in files (default agent + bundled skills) if absent.
        crate::seed::seed_workspace(&paths);

        // Load settings, honoring config overrides.
        let mut settings = load_settings(&paths);
        if let Some(agent) = &config.default_agent_id {
            settings.default_agent_id = agent.clone();
        }

        // Working directories.
        let default_work_dir = settings
            .default_work_dir
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let workdirs = Arc::new(WorkDirManager::new(default_work_dir));
        for d in config.work_dirs.iter().chain(settings.work_dirs.iter()) {
            let _ = workdirs.register(d);
        }

        let settings_arc = Arc::new(RwLock::new(settings));

        let skills = SkillRegistry::new(paths.clone());
        skills.register_native(Arc::new(CurrentTimeSkill));

        let tool_registry = Arc::new(fastmind::ToolRegistry::new());
        register_builtin_tools(&tool_registry);
        for tool in &config.extra_tools {
            tool_registry.add(tool.name.clone(), tool.clone());
        }

        let ctx = ToolContext::new(
            paths.clone(),
            workdirs.clone(),
            settings_arc.clone(),
            skills.clone(),
        );
        let runtime = AgentRuntime::new(
            paths.clone(),
            workdirs.clone(),
            settings_arc.clone(),
            skills.clone(),
            tool_registry.clone(),
        );

        let graph = build_graph(runtime.clone(), ctx.clone());

        let app = fastmind::App::new();
        app.register_graph("main", graph);
        let api = fastmind::FastMindAPI::new(app);

        let claw = FastClaw {
            api,
            session_store: SessionStore::new(paths.sessions_db()),
            paths,
            workdirs,
            settings: settings_arc,
            skills,
            tool_registry,
            ctx,
            runtime,
        };
        // Restore persisted per-session work-dir bindings from sessions.json.
        claw.restore_session_workdirs();
        claw
    }

    /// Re-bind every session that has a persisted `work_dir` entry.
    fn restore_session_workdirs(&self) {
        let sessions = self.session_store.load();
        if let Some(obj) = sessions.as_object() {
            for (sid, entry) in obj {
                if let Some(dir) = entry.get("work_dir").and_then(|v| v.as_str()) {
                    if !dir.is_empty() {
                        let _ = self.workdirs.bind(sid, std::path::Path::new(dir));
                    }
                }
            }
        }
    }

    // ── lifecycle ───────────────────────────────────────────────────────────

    pub async fn start(&self) -> Result<()> {
        self.api.start().await;
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        self.api.stop().await;
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.api.is_running()
    }

    // ── chat ────────────────────────────────────────────────────────────────

    /// Push a user message (non-blocking; results flow through the event stream).
    pub async fn chat(&self, session_id: &str, text: &str) -> Result<()> {
        let message_id = format!("msg_{}", &Uuid::new_v4().simple().to_string()[..12]);
        let event = fastmind::Event::with_id(
            fastmind::EventType::USER_MESSAGE,
            json!({ "text": text, "message_id": message_id }),
            session_id,
            message_id.clone(),
        );
        self.ensure_session_registered(session_id);
        self.api.push_event(session_id, event, "main").await?;
        Ok(())
    }

    /// Push a user message with attached images (multimodal).
    pub async fn chat_with_images(
        &self,
        session_id: &str,
        text: &str,
        images: &[crate::llm::ImagePart],
    ) -> Result<()> {
        let message_id = format!("msg_{}", &Uuid::new_v4().simple().to_string()[..12]);
        let imgs: Vec<Value> = images
            .iter()
            .filter_map(|i| serde_json::to_value(i).ok())
            .collect();
        let event = fastmind::Event::with_id(
            fastmind::EventType::USER_MESSAGE,
            json!({ "text": text, "message_id": message_id, "images": imgs }),
            session_id,
            message_id.clone(),
        );
        self.ensure_session_registered(session_id);
        self.api.push_event(session_id, event, "main").await?;
        Ok(())
    }

    /// Convenience streaming chat: push a message and collect the reply.
    pub async fn run_chat(
        &self,
        session_id: &str,
        text: &str,
        on_chunk: Option<Arc<dyn Fn(String) + Send + Sync>>,
        on_end: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> String {
        let _ = self.chat(session_id, text).await;
        let mut full = String::new();
        if let Ok(mut stream) = self.stream_events(session_id) {
            use futures::StreamExt;
            while let Some(ev) = stream.next().await {
                match ev.type_.as_str() {
                    "stream.chunk" => {
                        let delta = ev
                            .payload
                            .get("delta")
                            .and_then(|d| d.as_str())
                            .unwrap_or("")
                            .to_string();
                        full.push_str(&delta);
                        if let Some(cb) = &on_chunk {
                            cb(delta);
                        }
                    }
                    "stream.end" => {
                        if let Some(cb) = &on_end {
                            cb(full.clone());
                        }
                        break;
                    }
                    "stream.error" => break,
                    _ => {}
                }
            }
        }
        full
    }

    /// A multi-consumer event stream over a session.
    pub fn stream_events(&self, session_id: &str) -> Result<fastmind::EventStream> {
        self.api.stream_events(session_id, None).map_err(Into::into)
    }

    pub async fn stop_session(&self, session_id: &str) -> Result<()> {
        if let Some(session) = self.api.get_session(session_id) {
            session.stop().await;
            Ok(())
        } else {
            Err(Error::NotFound(format!("Session '{session_id}' not found")))
        }
    }

    // ── sessions ────────────────────────────────────────────────────────────

    fn ensure_session_registered(&self, session_id: &str) {
        let mut sessions = self.session_store.load();
        if sessions.get(session_id).is_none() {
            let default_agent = self
                .settings
                .read()
                .map(|s| s.default_agent_id.clone())
                .unwrap_or_else(|_| "main_agent".into());
            sessions[session_id] = json!({
                "session_id": session_id,
                "agent_id": default_agent,
                "created_at": chrono::Utc::now().timestamp(),
                "last_active_time": chrono::Utc::now().timestamp(),
                "channel": "api",
            });
            self.session_store.save(&sessions);
        }
    }

    pub fn new_session(&self, agent_id: Option<&str>) -> String {
        let sid = format!("{:08x}", Uuid::new_v4().as_u128());
        let mut sessions = self.session_store.load();
        let default_agent = self
            .settings
            .read()
            .map(|s| s.default_agent_id.clone())
            .unwrap_or_else(|_| "main_agent".into());
        sessions[&sid] = json!({
            "session_id": sid,
            "agent_id": agent_id.unwrap_or(&default_agent),
            "created_at": chrono::Utc::now().timestamp(),
            "last_active_time": chrono::Utc::now().timestamp(),
            "channel": "api",
        });
        self.session_store.save(&sessions);
        sid
    }

    pub fn list_sessions(&self) -> Vec<Value> {
        self.session_store
            .load()
            .as_object()
            .map(|obj| obj.values().cloned().collect())
            .unwrap_or_default()
    }

    pub fn get_session(&self, session_id: &str) -> Option<Value> {
        self.session_store.load().get(session_id).cloned()
    }

    pub async fn delete_session(&self, session_id: &str) {
        let mut sessions = self.session_store.load();
        if let Some(obj) = sessions.as_object_mut() {
            obj.remove(session_id);
        }
        self.session_store.save(&sessions);
        self.api.delete_session(session_id).await;
        let dir = self.paths.session_dir(session_id);
        let _ = std::fs::remove_dir_all(dir);
        self.workdirs.unbind(session_id);
    }

    // ── state / history ─────────────────────────────────────────────────────

    pub async fn get_state(&self, session_id: &str) -> Option<Value> {
        self.api.get_state(session_id).await.map(|s| s.to_value())
    }

    pub fn get_messages(&self, session_id: &str) -> Vec<Value> {
        crate::session::load_messages_from_jsonl(&self.paths.sessions_dir(), session_id)
    }

    pub fn clear_messages(&self, session_id: &str) {
        let file = self.paths.session_messages_file(session_id);
        let _ = std::fs::remove_file(file);
    }

    // ── skills / agents / settings ──────────────────────────────────────────

    pub fn list_skills(&self) -> Vec<crate::skills::SkillInfo> {
        self.skills.list()
    }

    pub fn list_agents(&self) -> Vec<String> {
        config::list_agents(&self.paths)
    }

    pub fn get_agent(&self, agent_id: &str) -> AgentConfig {
        config::load_agent_config(&self.paths, agent_id)
    }

    pub fn get_settings(&self) -> Settings {
        self.runtime.settings()
    }

    pub fn update_settings(&self, settings: Settings) {
        save_settings(&self.paths, &settings);
        self.ctx.set_settings(settings.clone());
        self.runtime.set_settings(settings);
    }

    // ── working directories ─────────────────────────────────────────────────

    /// Persist the current settings to `data/settings.json`.
    fn persist_settings(&self) -> Result<()> {
        let settings = self
            .settings
            .read()
            .map_err(|_| Error::WorkDir("lock poisoned".into()))?
            .clone();
        save_settings(&self.paths, &settings);
        Ok(())
    }

    /// Register a working directory into the trusted set and persist the
    /// global registered list to `settings.json` (survives restart).
    pub fn register_work_dir(&self, dir: &std::path::Path) -> Result<()> {
        self.workdirs.register(dir)?;
        {
            let mut guard = self
                .settings
                .write()
                .map_err(|_| Error::WorkDir("lock poisoned".into()))?;
            guard.work_dirs = self.workdirs.list();
        }
        self.persist_settings()
    }

    pub fn unregister_work_dir(&self, dir: &std::path::Path) -> Result<()> {
        self.workdirs.unregister(dir)?;
        {
            let mut guard = self
                .settings
                .write()
                .map_err(|_| Error::WorkDir("lock poisoned".into()))?;
            guard.work_dirs = self.workdirs.list();
        }
        self.persist_settings()
    }

    pub fn list_work_dirs(&self) -> Vec<PathBuf> {
        self.workdirs.list()
    }

    /// Set the global default working directory and persist it to
    /// `settings.json` (survives restart).
    pub fn set_default_work_dir(&self, dir: &std::path::Path) -> Result<()> {
        self.workdirs.set_default(dir)?;
        let default = self.workdirs.default();
        {
            let mut guard = self
                .settings
                .write()
                .map_err(|_| Error::WorkDir("lock poisoned".into()))?;
            guard.default_work_dir = Some(default.clone());
        }
        self.persist_settings()
    }

    pub fn default_work_dir(&self) -> PathBuf {
        self.workdirs.default()
    }

    /// Bind a session to a working directory and persist the binding into
    /// `sessions.json` (`work_dir` field) so it survives restart.
    pub fn bind_work_dir(&self, session_id: &str, dir: &std::path::Path) -> Result<()> {
        self.workdirs.bind(session_id, dir)?;
        self.ensure_session_registered(session_id);
        let mut sessions = self.session_store.load();
        if let Some(entry) = sessions.get_mut(session_id) {
            if let Some(obj) = entry.as_object_mut() {
                obj.insert(
                    "work_dir".to_string(),
                    json!(dir.to_string_lossy().into_owned()),
                );
            }
            self.session_store.save(&sessions);
        }
        Ok(())
    }

    /// Unbind a session and drop its persisted `work_dir` entry.
    pub fn unbind_work_dir(&self, session_id: &str) {
        self.workdirs.unbind(session_id);
        let mut sessions = self.session_store.load();
        if let Some(entry) = sessions.get_mut(session_id) {
            if let Some(obj) = entry.as_object_mut() {
                obj.remove("work_dir");
            }
            self.session_store.save(&sessions);
        }
    }

    pub fn resolve_work_dir(&self, session_id: &str, agent_id: Option<&str>) -> PathBuf {
        let cfg = agent_id
            .map(|a| config::load_agent_config(&self.paths, a))
            .unwrap_or_else(|| config::load_agent_config(&self.paths, "main_agent"));
        self.workdirs.resolve(session_id, &cfg.work_dirs)
    }

    // ── internal accessors (for tests / advanced hosts) ─────────────────────

    pub fn paths(&self) -> &WorkspacePaths {
        &self.paths
    }

    pub fn tool_registry(&self) -> Arc<fastmind::ToolRegistry> {
        self.tool_registry.clone()
    }

    /// Inject a mock LLM provider (for tests / no-key hosts).
    #[doc(hidden)]
    pub fn set_mock_provider(&self, provider: Arc<dyn crate::llm::LlmProvider>) {
        self.runtime.set_mock_provider(provider);
    }
}

/// A built-in native skill: current time.
struct CurrentTimeSkill;

#[async_trait]
impl Skill for CurrentTimeSkill {
    fn name(&self) -> &str {
        "current_time"
    }

    fn description(&self) -> &str {
        "Get the current date and time"
    }

    async fn execute(&self, _params: Value) -> Result<String> {
        Ok(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string())
    }
}

/// A boxed event stream (type alias for convenience).
pub type EventStream = fastmind::EventStream;
