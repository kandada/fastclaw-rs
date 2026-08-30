// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Settings / AgentConfig loading.

use std::path::Path;

use fastclaw::config::{
    list_agents, load_agent_config, load_agent_personality, load_settings,
    resolve_session_agent_id, save_settings,
};
use fastclaw::{LlmGateway, Settings, WorkspacePaths};
use serde_json::json;

fn paths(root: &Path) -> WorkspacePaths {
    let p = WorkspacePaths::new(root.to_path_buf());
    p.ensure().unwrap();
    p
}

#[test]
fn settings_default_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    let s = load_settings(&paths(dir.path()));
    assert_eq!(s.default_agent_id, "main_agent");
    assert_eq!(s.run_shell_timeout, 60);
    assert_eq!(s.run_skills_timeout, 60);
    assert_eq!(s.schema_version, 1);
}

#[test]
fn settings_merge_partial_file() {
    let dir = tempfile::tempdir().unwrap();
    let p = paths(dir.path());
    std::fs::write(
        p.settings_file(),
        json!({ "default_agent_id": "custom", "run_shell_timeout": 30 }).to_string(),
    )
    .unwrap();
    let s = load_settings(&p);
    assert_eq!(s.default_agent_id, "custom");
    assert_eq!(s.run_shell_timeout, 30);
    // Untouched fields keep defaults.
    assert_eq!(s.run_skills_timeout, 60);
}

#[test]
fn settings_save_load_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let p = paths(dir.path());
    let s = Settings {
        default_agent_id: "x".into(),
        default_work_dir: Some(dir.path().join("wd")),
        work_dirs: vec![dir.path().join("a"), dir.path().join("b")],
        ..Settings::default()
    };
    save_settings(&p, &s);

    let loaded = load_settings(&p);
    assert_eq!(loaded.default_agent_id, "x");
    assert_eq!(loaded.default_work_dir, Some(dir.path().join("wd")));
    assert_eq!(loaded.work_dirs.len(), 2);
}

#[test]
fn agent_config_default_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let c = load_agent_config(&paths(dir.path()), "nonexistent");
    assert_eq!(c.name, "nonexistent");
    assert_eq!(c.llm.gateway, LlmGateway::OpenAi);
    assert!(c.work_dirs.is_empty());
}

#[test]
fn agent_config_from_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let p = paths(dir.path());
    let agent_dir = p.agents_dir().join("my_agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(
        agent_dir.join("metadata.json"),
        json!({
            "name": "my_agent",
            "description": "desc",
            "llm": { "gateway": "anthropic", "model": "claude-3", "api_key": "k", "base_url": "https://api.anthropic.com" },
            "context": { "max_tokens": 100000, "unload_threshold_tokens": 1000 },
            "extra_workspaces": ["/a", "/b"]
        })
        .to_string(),
    )
    .unwrap();

    let c = load_agent_config(&p, "my_agent");
    assert_eq!(c.llm.gateway, LlmGateway::Anthropic);
    assert_eq!(c.llm.model, "claude-3");
    assert_eq!(c.context.unload_threshold_tokens, 1000);
    assert_eq!(c.context.max_tokens, 100000);
    // `extra_workspaces` alias maps to `work_dirs`.
    assert_eq!(c.work_dirs.len(), 2);
    assert_eq!(c.work_dirs[0], std::path::PathBuf::from("/a"));
}

#[test]
fn agent_config_work_dirs_modern_name() {
    let dir = tempfile::tempdir().unwrap();
    let p = paths(dir.path());
    let agent_dir = p.agents_dir().join("modern");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(
        agent_dir.join("metadata.json"),
        json!({ "name": "modern", "work_dirs": ["/x"] }).to_string(),
    )
    .unwrap();
    let c = load_agent_config(&p, "modern");
    assert_eq!(c.work_dirs, vec![std::path::PathBuf::from("/x")]);
}

#[test]
fn personality_concatenates_three_files() {
    let dir = tempfile::tempdir().unwrap();
    let p = paths(dir.path());
    let d = p.agents_dir().join("a");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("SOUL.md"), "soul text").unwrap();
    std::fs::write(d.join("USER.md"), "user text").unwrap();
    std::fs::write(d.join("AGENT.md"), "agent text").unwrap();
    let pers = load_agent_personality(&p, "a");
    assert!(pers.contains("## SOUL"));
    assert!(pers.contains("soul text"));
    assert!(pers.contains("## USER"));
    assert!(pers.contains("user text"));
    assert!(pers.contains("## AGENT"));
    assert!(pers.contains("agent text"));
}

#[test]
fn personality_empty_when_no_files() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(load_agent_personality(&paths(dir.path()), "a"), "");
}

#[test]
fn list_agents_sorted() {
    let dir = tempfile::tempdir().unwrap();
    let p = paths(dir.path());
    std::fs::create_dir_all(p.agents_dir().join("zeta")).unwrap();
    std::fs::create_dir_all(p.agents_dir().join("alpha")).unwrap();
    assert_eq!(
        list_agents(&p),
        vec!["alpha".to_string(), "zeta".to_string()]
    );
}

#[test]
fn resolve_session_agent_id_default_and_bound() {
    let dir = tempfile::tempdir().unwrap();
    let p = paths(dir.path());
    // No session → default.
    assert_eq!(resolve_session_agent_id(&p, "s1"), "main_agent");

    // Session bound to a specific agent.
    let store = fastclaw::workspace::SessionStore::new(p.sessions_db());
    let mut s = store.load();
    s["s1"] = json!({ "agent_id": "my_agent" });
    store.save(&s);
    assert_eq!(resolve_session_agent_id(&p, "s1"), "my_agent");
}
