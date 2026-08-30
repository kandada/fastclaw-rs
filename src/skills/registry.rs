// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Skill registry: discovery of `SKILL.md` meta-skills and native Rust skills.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use regex::Regex;
use serde_json::Value;

use crate::error::{Error, Result};
use crate::workspace::WorkspacePaths;

/// What kind a skill is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillKind {
    /// A SKILL.md instruction document (the agent reads it and acts itself).
    Meta,
    /// A native Rust implementation of [`Skill`].
    Native,
}

/// Metadata about a discovered skill.
#[derive(Debug, Clone)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub kind: SkillKind,
}

/// A native skill implemented in Rust.
#[async_trait]
pub trait Skill: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    async fn execute(&self, params: Value) -> Result<String>;
}

/// Locate `SKILL.md` case-insensitively within a skill directory.
pub fn find_skill_md(skill_dir: &Path) -> Option<PathBuf> {
    let direct = skill_dir.join("SKILL.md");
    if direct.exists() {
        return Some(direct);
    }
    if let Ok(entries) = std::fs::read_dir(skill_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file()
                && p.file_name().map(|n| n.to_string_lossy().to_lowercase())
                    == Some("skill.md".into())
            {
                return Some(p);
            }
        }
    }
    None
}

/// Parse a skill description: prefer YAML frontmatter `description`, else the
/// `## Description` section.
pub fn parse_skill_description(content: &str) -> String {
    // YAML frontmatter `description:` (single or double quoted).
    let fm_re =
        Regex::new(r"(?s)^\ufeff?\s*---\s*\n(.*?)\n---\s*(?:\n|$)").expect("valid fm regex");
    if let Some(caps) = fm_re.captures(content) {
        let fm = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let desc_re = Regex::new(r"(?m)^\s*description\s*:\s*(.+?)\s*$").expect("valid desc regex");
        if let Some(d) = desc_re.captures(fm) {
            let desc = d.get(1).map(|m| m.as_str()).unwrap_or("").trim();
            let desc = desc.trim_matches('"').trim_matches('\'').trim();
            if !desc.is_empty() {
                return desc.to_string();
            }
        }
    }
    // `## Description` section.
    let mut in_desc = false;
    let mut collected: Vec<String> = Vec::new();
    for line in content.lines() {
        let s = line.trim();
        if s.to_lowercase().starts_with("## description") {
            in_desc = true;
            continue;
        }
        if in_desc {
            if s.starts_with("## ") {
                break;
            }
            if s.is_empty() {
                if !collected.is_empty() {
                    break;
                }
                continue;
            }
            collected.push(s.to_string());
        }
    }
    if !collected.is_empty() {
        return collected.join(" ");
    }
    String::new()
}

/// List resource files inside a skill dir (excluding SKILL.md).
pub fn list_skill_resources(skill_dir: &Path) -> Vec<String> {
    fn collect(dir: &Path, base: &Path, out: &mut Vec<String>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    let name = p
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if name.starts_with('.') || name == "__pycache__" {
                        continue;
                    }
                    collect(&p, base, out);
                } else if p.is_file() {
                    if p.file_name().map(|n| n.to_string_lossy().to_lowercase())
                        == Some("skill.md".into())
                    {
                        continue;
                    }
                    if let Ok(rel) = p.strip_prefix(base) {
                        out.push(rel.to_string_lossy().into_owned());
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    collect(skill_dir, skill_dir, &mut out);
    out.sort();
    out
}

/// The skill registry: discovers meta-skills and hosts native skills.
pub struct SkillRegistry {
    paths: WorkspacePaths,
    native: RwLock<HashMap<String, Arc<dyn Skill>>>,
}

impl SkillRegistry {
    pub fn new(paths: WorkspacePaths) -> Arc<Self> {
        Arc::new(SkillRegistry {
            paths,
            native: RwLock::new(HashMap::new()),
        })
    }

    /// Register a native (Rust-implemented) skill.
    pub fn register_native(&self, skill: Arc<dyn Skill>) {
        if let Ok(mut guard) = self.native.write() {
            guard.insert(skill.name().to_string(), skill);
        }
    }

    fn discover_meta(dir: &Path) -> Vec<SkillInfo> {
        fn collect(
            dir: &Path,
            seen: &mut std::collections::HashSet<String>,
            out: &mut Vec<SkillInfo>,
        ) {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_dir() {
                        let name = p
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        if name.starts_with('_') {
                            continue;
                        }
                        if let Some(md) = find_skill_md(&p) {
                            let key = md
                                .parent()
                                .map(|d| d.to_string_lossy().to_lowercase())
                                .unwrap_or_default();
                            if seen.insert(key) {
                                let content = std::fs::read_to_string(&md).unwrap_or_default();
                                let desc = parse_skill_description(&content);
                                out.push(SkillInfo {
                                    name,
                                    description: if desc.is_empty() {
                                        "a skill".to_string()
                                    } else {
                                        desc
                                    },
                                    path: p.clone(),
                                    kind: SkillKind::Meta,
                                });
                            }
                        }
                        collect(&p, seen, out);
                    }
                }
            }
        }
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        collect(dir, &mut seen, &mut out);
        out
    }

    /// List all skills: bundled meta-skills + user meta-skills + native skills.
    pub fn list(&self) -> Vec<SkillInfo> {
        let mut out = Self::discover_meta(&self.paths.bundled_skills_dir());
        out.extend(Self::discover_meta(&self.paths.user_skills_dir()));
        if let Ok(guard) = self.native.read() {
            for skill in guard.values() {
                out.push(SkillInfo {
                    name: skill.name().to_string(),
                    description: skill.description().to_string(),
                    path: PathBuf::new(),
                    kind: SkillKind::Native,
                });
            }
        }
        out
    }

    pub fn get(&self, name: &str) -> Option<SkillInfo> {
        self.list().into_iter().find(|s| s.name == name)
    }

    /// A `- name: description` list string for the system prompt.
    pub fn skills_list_string(&self) -> String {
        let skills = self.list();
        if skills.is_empty() {
            return "- (No built-in skills)".to_string();
        }
        skills
            .iter()
            .map(|s| format!("- {}: {}", s.name, s.description))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Execute a skill. Meta skills return the SKILL.md guidance + resources;
    /// native skills run their Rust implementation.
    pub async fn execute(&self, name: &str, params: Value, timeout_secs: u64) -> Result<String> {
        if let Some(info) = self.get(name) {
            match info.kind {
                SkillKind::Native => {
                    let skill = {
                        let guard = self
                            .native
                            .read()
                            .map_err(|_| Error::Skill("lock poisoned".into()))?;
                        guard.get(name).cloned()
                    };
                    if let Some(s) = skill {
                        return execute_with_timeout(s, params, timeout_secs).await;
                    }
                }
                SkillKind::Meta => {
                    return Ok(render_meta_skill(&info.name, &info.path, &params));
                }
            }
        }
        Err(Error::Skill(format!("Skill '{name}' not found")))
    }
}

/// Render a meta-skill into actionable guidance for the agent.
fn render_meta_skill(name: &str, dir: &Path, params: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(md) = find_skill_md(dir) {
        parts.push(
            std::fs::read_to_string(&md)
                .unwrap_or_default()
                .trim_end()
                .to_string(),
        );
    } else {
        return format!("Error: Skill '{name}' not found at {}", dir.display());
    }
    let resources = list_skill_resources(dir);
    if !resources.is_empty() {
        parts.push(
            "\n## Available Resources (read on demand with run_shell)\n".to_string()
                + &resources
                    .iter()
                    .map(|r| format!("- {r}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
        );
    }
    if let Some(obj) = params.as_object() {
        if !obj.is_empty() {
            let keys: Vec<&str> = obj.keys().map(|s| s.as_str()).collect();
            parts.push(format!(
                "\n(Note: this is a document-only skill with no executable script; the provided params {} were not consumed. Follow the instructions above and perform the steps yourself via run_shell.)",
                keys.join(", ")
            ));
        }
    }
    parts.join("\n")
}

async fn execute_with_timeout(
    skill: Arc<dyn Skill>,
    params: Value,
    timeout_secs: u64,
) -> Result<String> {
    let fut = skill.execute(params);
    match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), fut).await {
        Ok(res) => res,
        Err(_) => Err(Error::Skill(format!(
            "Skill '{}' timed out ({timeout_secs}s)",
            skill.name()
        ))),
    }
}
