// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Skill discovery / parsing / execution.

use std::sync::Arc;

use async_trait::async_trait;
use fastclaw::skills::registry::{parse_skill_description, Skill};
use fastclaw::SkillKind;
use serde_json::{json, Value};

mod common;
use common::make_skills;

struct EchoSkill;

#[async_trait]
impl Skill for EchoSkill {
    fn name(&self) -> &str {
        "echo_skill"
    }
    fn description(&self) -> &str {
        "Echo back params"
    }
    async fn execute(&self, params: Value) -> fastclaw::Result<String> {
        Ok(params
            .get("msg")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string())
    }
}

#[test]
fn parse_frontmatter_description() {
    let content = "---\ndescription: \"Do a thing\"\n---\n\n## Body\n...";
    assert_eq!(parse_skill_description(content), "Do a thing");
}

#[test]
fn parse_section_description() {
    let content = "## Description\nGet the current time\n\n## Parameters\nNone";
    assert_eq!(parse_skill_description(content), "Get the current time");
}

#[test]
fn discover_meta_skills() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let bundled = root.join("skills").join("bundled");
    std::fs::create_dir_all(bundled.join("hello")).unwrap();
    std::fs::write(
        bundled.join("hello").join("SKILL.md"),
        "## Description\nSay hello\n",
    )
    .unwrap();
    // A hidden/_prefixed skill is ignored.
    std::fs::create_dir_all(bundled.join("_secret")).unwrap();
    std::fs::write(
        bundled.join("_secret").join("SKILL.md"),
        "## Description\nhidden\n",
    )
    .unwrap();

    let reg = make_skills(root);
    let list = reg.list();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "hello");
    assert_eq!(list[0].kind, SkillKind::Meta);
    assert_eq!(list[0].description, "Say hello");
}

#[tokio::test]
async fn native_skill_registered_and_executed() {
    let dir = tempfile::tempdir().unwrap();
    let reg = make_skills(dir.path());
    reg.register_native(Arc::new(EchoSkill));

    let list = reg.list();
    assert!(list.iter().any(|s| s.name == "echo_skill"));

    let out = reg
        .execute("echo_skill", json!({"msg": "hi"}), 5)
        .await
        .unwrap();
    assert_eq!(out, "hi");
}

#[tokio::test]
async fn meta_skill_render_returns_guidance() {
    let dir = tempfile::tempdir().unwrap();
    let bundled = dir.path().join("skills").join("bundled");
    std::fs::create_dir_all(bundled.join("guide")).unwrap();
    std::fs::write(
        bundled.join("guide").join("SKILL.md"),
        "## Description\nFollow these steps\n\nUse run_shell to do X\n",
    )
    .unwrap();
    std::fs::write(bundled.join("guide").join("notes.txt"), "resource").unwrap();

    let reg = make_skills(dir.path());
    let out = reg.execute("guide", json!({}), 5).await.unwrap();
    assert!(out.contains("Follow these steps"));
    assert!(out.contains("notes.txt"));
}

#[tokio::test]
async fn missing_skill_errors() {
    let dir = tempfile::tempdir().unwrap();
    let reg = make_skills(dir.path());
    assert!(reg.execute("nope", json!({}), 5).await.is_err());
}

#[test]
fn skills_list_string_format() {
    let dir = tempfile::tempdir().unwrap();
    let reg = make_skills(dir.path());
    reg.register_native(Arc::new(EchoSkill));
    let s = reg.skills_list_string();
    assert!(s.contains("- echo_skill: Echo back params"));
}

#[test]
fn find_skill_md_case_insensitive() {
    use fastclaw::skills::registry::find_skill_md;
    let dir = tempfile::tempdir().unwrap();
    let skill = dir.path().join("lower");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(skill.join("skill.md"), "## Description\nlowercase").unwrap();
    let md = find_skill_md(&skill).expect("should find skill.md case-insensitively");
    // The discovered file exists and holds the content (on case-insensitive
    // filesystems the direct `SKILL.md` path resolves to the same file).
    assert!(md.exists());
    assert!(std::fs::read_to_string(&md).unwrap().contains("lowercase"));
}

#[test]
fn list_skill_resources_nested_and_excludes_skill_md() {
    use fastclaw::skills::registry::list_skill_resources;
    let dir = tempfile::tempdir().unwrap();
    let skill = dir.path().join("s");
    std::fs::create_dir_all(skill.join("sub")).unwrap();
    std::fs::write(skill.join("SKILL.md"), "x").unwrap();
    std::fs::write(skill.join("a.txt"), "x").unwrap();
    std::fs::write(skill.join("sub").join("b.txt"), "x").unwrap();

    let resources = list_skill_resources(&skill);
    assert!(resources.contains(&"a.txt".to_string()));
    assert!(resources.contains(&"sub/b.txt".to_string()));
    assert!(!resources
        .iter()
        .any(|r| r.to_lowercase().contains("skill.md")));
}
