// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Workspace bootstrap: seed built-in files from `workspace_seed/`.
//!
//! Mirrors the Python `bootstrap.copy_seed_files` contract: on first startup
//! (and on version upgrades that add new files) the seed content is copied
//! into the workspace with file-level protection — existing files are never
//! overwritten, so user edits are always respected.
//!
//! Seed sources live as real tracked files under `<crate>/workspace_seed/`
//! and are embedded into the binary at compile time via `include_str!`, so
//! desktop / mobile / embedded hosts stay self-contained (no runtime path
//! resolution, no external file tree required).

use crate::workspace::WorkspacePaths;

/// Workspace-relative files to seed: `(relative path, embedded content)`.
/// Relative paths mirror the `workspace_seed/` tree, e.g.
/// `skills/bundled/<name>/SKILL.md` or `data/agents/<id>/SOUL.md`.
pub const SEED_FILES: &[(&str, &str)] = &[
    // Bundled skills.
    (
        "skills/bundled/skill_creator/SKILL.md",
        include_str!("../workspace_seed/skills/bundled/skill_creator/SKILL.md"),
    ),
    // User skill template (prefix `_` keeps it hidden from the registry).
    (
        "skills/user/_template/SKILL.md",
        include_str!("../workspace_seed/skills/user/_template/SKILL.md"),
    ),
    // Default agent personality + config.
    (
        "data/agents/main_agent/SOUL.md",
        include_str!("../workspace_seed/data/agents/main_agent/SOUL.md"),
    ),
    (
        "data/agents/main_agent/USER.md",
        include_str!("../workspace_seed/data/agents/main_agent/USER.md"),
    ),
    (
        "data/agents/main_agent/AGENT.md",
        include_str!("../workspace_seed/data/agents/main_agent/AGENT.md"),
    ),
    (
        "data/agents/main_agent/metadata.json",
        include_str!("../workspace_seed/data/agents/main_agent/metadata.json"),
    ),
];

/// Seed built-in files into the workspace. Each missing file is written
/// atomically (tmp + rename); existing files are never overwritten.
/// Returns the number of files written.
pub fn seed_workspace(paths: &WorkspacePaths) -> u32 {
    let mut copied = 0u32;
    for (rel, content) in SEED_FILES {
        let dst = paths.root.join(rel);
        if dst.exists() {
            continue;
        }
        if let Some(parent) = dst.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::warn!("seed: create dir {} failed: {e}", parent.display());
                continue;
            }
        }
        let tmp = dst.with_extension("tmp");
        if let Err(e) = std::fs::write(&tmp, content) {
            tracing::warn!("seed: write {} failed: {e}", tmp.display());
            continue;
        }
        if std::fs::rename(&tmp, &dst).is_err() {
            let _ = std::fs::remove_file(&tmp);
            continue;
        }
        copied += 1;
    }
    if copied > 0 {
        tracing::info!("seeded {copied} file(s) from built-in workspace");
    }
    copied
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn paths(root: &Path) -> WorkspacePaths {
        let p = WorkspacePaths::new(root.to_path_buf());
        p.ensure().unwrap();
        p
    }

    #[test]
    fn seed_files_are_well_formed() {
        assert!(!SEED_FILES.is_empty());
        for (rel, content) in SEED_FILES {
            assert!(!rel.starts_with('/'), "relative path required: {rel}");
            assert!(
                !rel.starts_with(".."),
                "path must stay under workspace root: {rel}"
            );
            assert!(!content.trim().is_empty(), "empty seed content: {rel}");
        }
        // No duplicate targets.
        let mut seen = std::collections::HashSet::new();
        for (rel, _) in SEED_FILES {
            assert!(seen.insert(*rel), "duplicate seed target: {rel}");
        }
    }

    #[test]
    fn seed_workspace_creates_all_files() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        let n = seed_workspace(&p);
        assert_eq!(n as usize, SEED_FILES.len());

        for (rel, content) in SEED_FILES {
            let f = p.root.join(rel);
            assert!(f.exists(), "missing seeded file: {rel}");
            assert_eq!(std::fs::read_to_string(&f).unwrap(), *content);
        }
    }

    #[test]
    fn seed_workspace_does_not_overwrite() {
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

        let n = seed_workspace(&p);
        assert_eq!(n as usize, SEED_FILES.len() - 2);
        assert_eq!(std::fs::read_to_string(&soul).unwrap(), "# CUSTOM SOUL\n");
        assert_eq!(std::fs::read_to_string(&skill).unwrap(), "# CUSTOM SKILL\n");
        // The rest were still created.
        assert!(p.agents_dir().join("main_agent").join("AGENT.md").exists());
        assert!(p
            .user_skills_dir()
            .join("_template")
            .join("SKILL.md")
            .exists());
    }
}
