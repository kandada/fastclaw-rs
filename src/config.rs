// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Settings and agent-configuration loading.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::llm::LlmGateway;
use crate::workspace::WorkspacePaths;

/// Global settings (`data/settings.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub default_agent_id: String,
    pub run_shell_timeout: u64,
    pub run_skills_timeout: u64,
    pub stream_chunk_timeout: u64,
    /// Default working directory when a task specifies none.
    pub default_work_dir: Option<PathBuf>,
    /// Globally registered working directories.
    pub work_dirs: Vec<PathBuf>,
    /// Format version marker (see design doc §2.4.2).
    pub schema_version: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            default_agent_id: "main_agent".to_string(),
            run_shell_timeout: 60,
            run_skills_timeout: 60,
            stream_chunk_timeout: 60,
            default_work_dir: None,
            work_dirs: vec![],
            schema_version: 1,
        }
    }
}

/// Per-agent LLM configuration (`metadata.json` `llm` field).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LlmConfig {
    #[serde(default)]
    pub gateway: LlmGateway,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub timeout: Option<u64>,
    #[serde(default)]
    pub max_tokens: Option<u64>,
    #[serde(default)]
    pub multimodal: bool,
    /// Anthropic extended thinking. `Some(false)` disables it explicitly;
    /// `None`/`Some(true)` defaults to `adaptive`.
    #[serde(default)]
    pub enable_thinking: Option<bool>,
    /// `budget_tokens` used when falling back to `thinking.type = enabled`.
    #[serde(default)]
    pub thinking_budget_tokens: Option<u64>,
}

/// Per-agent configuration (`metadata.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub llm: LlmConfig,
    #[serde(default)]
    pub context: ContextConfig,
    /// Working directories the agent may write into (a.k.a. `extra_workspaces`).
    #[serde(default, alias = "extra_workspaces")]
    pub work_dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContextConfig {
    #[serde(default)]
    pub max_tokens: u64,
    #[serde(default)]
    pub unload_threshold_tokens: u64,
}

impl AgentConfig {
    pub fn default_for(name: &str) -> Self {
        AgentConfig {
            name: name.to_string(),
            description: String::new(),
            llm: LlmConfig {
                gateway: LlmGateway::OpenAi,
                provider: "deepseek".to_string(),
                model: "deepseek-chat".to_string(),
                api_key: std::env::var("LLM_API_KEY").unwrap_or_default(),
                base_url: std::env::var("LLM_API_URL")
                    .unwrap_or_else(|_| "https://api.deepseek.com/v1".to_string()),
                timeout: None,
                max_tokens: None,
                multimodal: false,
                enable_thinking: None,
                thinking_budget_tokens: None,
            },
            context: ContextConfig {
                max_tokens: 80000,
                unload_threshold_tokens: 256000,
            },
            work_dirs: vec![],
        }
    }
}

fn read_json(path: &Path) -> Option<Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

/// Load global settings, applying defaults for missing fields.
pub fn load_settings(paths: &WorkspacePaths) -> Settings {
    let mut settings = Settings::default();
    if let Some(v) = read_json(&paths.settings_file()) {
        if let Ok(mut merged) = serde_json::to_value(Settings::default()) {
            if let (Some(base), Some(user)) = (merged.as_object_mut(), v.as_object()) {
                for (k, val) in user {
                    base.insert(k.clone(), val.clone());
                }
            }
            if let Ok(s) = serde_json::from_value::<Settings>(merged) {
                settings = s;
            }
        }
    }
    settings
}

pub fn save_settings(paths: &WorkspacePaths, settings: &Settings) {
    if let Some(parent) = paths.settings_file().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(s) = serde_json::to_string_pretty(settings) {
        let _ = std::fs::write(paths.settings_file(), s);
    }
}

/// Load an agent's `metadata.json`, falling back to a default config.
///
/// Seed agents ship with an empty `api_key`; when empty, the key (and, for the
/// OpenAI gateway, the base URL / model) is filled from the environment.
pub fn load_agent_config(paths: &WorkspacePaths, agent_id: &str) -> AgentConfig {
    let metadata = paths.agents_dir().join(agent_id).join("metadata.json");
    let mut config = if let Some(v) = read_json(&metadata) {
        serde_json::from_value::<AgentConfig>(v)
            .unwrap_or_else(|_| AgentConfig::default_for(agent_id))
    } else {
        AgentConfig::default_for(agent_id)
    };
    if config.llm.api_key.is_empty() {
        match config.llm.gateway {
            LlmGateway::OpenAi => {
                if let Ok(key) = std::env::var("LLM_API_KEY") {
                    if !key.is_empty() {
                        config.llm.api_key = key;
                    }
                }
                if let Ok(url) = std::env::var("LLM_API_URL") {
                    if !url.is_empty() {
                        config.llm.base_url = url;
                    }
                }
                if let Ok(model) = std::env::var("LLM_MODEL_NAME") {
                    if !model.is_empty() {
                        config.llm.model = model;
                    }
                }
            }
            LlmGateway::Anthropic => {
                if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
                    if !key.is_empty() {
                        config.llm.api_key = key;
                    }
                }
                if let Ok(model) = std::env::var("ANTHROPIC_MODEL") {
                    if !model.is_empty() {
                        config.llm.model = model;
                    }
                }
            }
        }
    }
    config
}

/// Load an agent's personality from `SOUL.md` / `USER.md` / `AGENT.md`.
pub fn load_agent_personality(paths: &WorkspacePaths, agent_id: &str) -> String {
    let dir = paths.agents_dir().join(agent_id);
    let mut parts = Vec::new();
    for (file, tag) in [
        ("SOUL.md", "SOUL"),
        ("USER.md", "USER"),
        ("AGENT.md", "AGENT"),
    ] {
        let path = dir.join(file);
        if let Ok(content) = std::fs::read_to_string(path) {
            let content = content.trim();
            if !content.is_empty() {
                parts.push(format!("\n\n## {tag}\n{content}"));
            }
        }
    }
    parts.join("")
}

/// List all agent ids under `data/agents/`.
pub fn list_agents(paths: &WorkspacePaths) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(paths.agents_dir()) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                out.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    out.sort();
    out
}

/// Resolve the agent id bound to a session, falling back to the default.
pub fn resolve_session_agent_id(paths: &WorkspacePaths, session_id: &str) -> String {
    let store = crate::workspace::SessionStore::new(paths.sessions_db());
    let sessions = store.load();
    if let Some(agent_id) = sessions
        .get(session_id)
        .and_then(|s| s.get("agent_id"))
        .and_then(|v| v.as_str())
    {
        if !agent_id.is_empty() {
            return agent_id.to_string();
        }
    }
    load_settings(paths).default_agent_id
}
