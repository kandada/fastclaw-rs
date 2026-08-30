// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Workspace bootstrap: built-in seed files are written to the workspace with
//! file-level no-overwrite protection, and the seeded content is loadable /
//! discoverable by the rest of the system.

use fastclaw::config::{list_agents, load_agent_config, load_agent_personality};
use fastclaw::seed::{seed_workspace, SEED_FILES};
use fastclaw::SkillKind;
use fastclaw::WorkspacePaths;
use serde_json::json;

mod common;
use common::{build_claw, make_tool_context};

fn paths(root: &std::path::Path) -> WorkspacePaths {
    let p = WorkspacePaths::new(root.to_path_buf());
    p.ensure().unwrap();
    p
}

#[test]
fn seed_workspace_creates_every_file_with_exact_content() {
    let dir = tempfile::tempdir().unwrap();
    let p = paths(dir.path());

    let copied = seed_workspace(&p);
    assert_eq!(copied as usize, SEED_FILES.len());

    for (rel, content) in SEED_FILES {
        let f = p.root.join(rel);
        assert!(f.exists(), "missing seeded file: {rel}");
        assert_eq!(
            std::fs::read_to_string(&f).unwrap(),
            *content,
            "content mismatch for {rel}"
        );
    }
}

#[test]
fn seed_workspace_does_not_overwrite_existing_files() {
    let dir = tempfile::tempdir().unwrap();
    let p = paths(dir.path());

    let soul = p.agents_dir().join("main_agent").join("SOUL.md");
    std::fs::create_dir_all(soul.parent().unwrap()).unwrap();
    std::fs::write(&soul, "# CUSTOM SOUL\n").unwrap();
    let skill = p
        .bundled_skills_dir()
        .join("skill_creator")
        .join("SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(&skill, "# CUSTOM SKILL\n").unwrap();

    let copied = seed_workspace(&p);
    assert_eq!(copied as usize, SEED_FILES.len() - 2);
    assert_eq!(std::fs::read_to_string(&soul).unwrap(), "# CUSTOM SOUL\n");
    assert_eq!(std::fs::read_to_string(&skill).unwrap(), "# CUSTOM SKILL\n");

    // Idempotent: a second run copies nothing.
    assert_eq!(seed_workspace(&p), 0);
}

#[test]
fn building_fastclaw_seeds_the_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let (claw, _) = build_claw(root, vec![]);

    // Default agent files.
    let agent_dir = claw.paths().agents_dir().join("main_agent");
    for file in ["SOUL.md", "USER.md", "AGENT.md", "metadata.json"] {
        assert!(agent_dir.join(file).exists(), "missing {file}");
    }
    // Bundled skill + user template.
    assert!(claw
        .paths()
        .bundled_skills_dir()
        .join("skill_creator")
        .join("SKILL.md")
        .exists());
    assert!(claw
        .paths()
        .user_skills_dir()
        .join("_template")
        .join("SKILL.md")
        .exists());
}

#[test]
fn seeded_default_agent_is_loadable() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let p = paths(root);
    seed_workspace(&p);

    // Config loads with an empty key and the expected id.
    let cfg = load_agent_config(&p, "main_agent");
    assert_eq!(cfg.name, "main_agent");
    assert_eq!(cfg.llm.api_key, "");

    // Personality concatenates the three seeded markdown files.
    let personality = load_agent_personality(&p, "main_agent");
    assert!(personality.contains("SOUL") && personality.contains("FastClaw"));
    assert!(personality.contains("USER") && personality.contains("User"));
    assert!(personality.contains("AGENT") && personality.contains("Guidelines"));

    // The agent shows up in the listing.
    assert!(list_agents(&p).contains(&"main_agent".to_string()));
}

#[test]
fn seeded_skill_creator_is_discovered_as_meta() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let p = paths(root);
    seed_workspace(&p);

    let reg = common::make_skills(root);
    let list = reg.list();
    let creator = list
        .iter()
        .find(|s| s.name == "skill_creator")
        .expect("skill_creator should be discovered");
    assert_eq!(creator.kind, SkillKind::Meta);
    assert!(creator
        .description
        .contains("how to create, update and optimize skills"));

    // The `_template` directory must stay hidden from the registry.
    assert!(!list.iter().any(|s| s.name == "_template"));
}

#[tokio::test]
async fn run_skills_tool_lists_and_inspects_seeded_skills() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let p = paths(root);
    seed_workspace(&p);
    let ctx = make_tool_context(root);

    // __list__
    let list = fastclaw::tools::skills::run_skills(&ctx, Some("__list__".into()), None, None)
        .await
        .unwrap();
    assert!(list.contains("skill_creator"), "list: {list}");

    // __info__
    let info = fastclaw::tools::skills::run_skills(
        &ctx,
        Some("__info__".into()),
        Some(json!({ "skill_name": "skill_creator" })),
        None,
    )
    .await
    .unwrap();
    assert!(info.contains("## Description"), "info: {info}");

    // Executing the meta-skill returns actionable guidance.
    let out = fastclaw::tools::skills::run_skills(
        &ctx,
        Some("skill_creator".into()),
        Some(json!({ "action": "create", "name": "my_skill" })),
        None,
    )
    .await
    .unwrap();
    assert!(out.contains("How to create a skill"), "exec: {out}");

    // Missing skill reports a clear error.
    let err = fastclaw::tools::skills::run_skills(&ctx, Some("nope".into()), None, None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("nope"), "err: {err}");
}
