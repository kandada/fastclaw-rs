// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Workspace root resolution and sub-path access.

use std::path::{Path, PathBuf};

use crate::error::Result;

/// The workspace root — where fastclaw keeps its metadata (sessions, agents,
/// skills, settings). Distinct from per-task *working directories*.
#[derive(Debug, Clone)]
pub struct WorkspacePaths {
    pub root: PathBuf,
}

impl WorkspacePaths {
    /// Resolve the workspace root with the documented priority chain:
    /// 1. explicitly injected root (mobile/embedded hosts);
    /// 2. `FASTCLAW_WORKSPACE` env var (explicit migration/sharing);
    /// 3. debug-only `FASTCLAW_DEV_WORKSPACE` baked by `build.rs`;
    /// 4. `~/.fastclaw-rs/workspace` (release/installed default).
    pub fn resolve(injected: Option<PathBuf>) -> PathBuf {
        if let Some(p) = injected {
            return p;
        }
        if let Ok(p) = std::env::var("FASTCLAW_WORKSPACE") {
            if !p.trim().is_empty() {
                return PathBuf::from(p);
            }
        }
        #[cfg(debug_assertions)]
        if let Ok(p) = std::env::var("FASTCLAW_DEV_WORKSPACE") {
            if !p.trim().is_empty() {
                return PathBuf::from(p);
            }
        }
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".fastclaw-rs")
            .join("workspace")
    }

    pub fn new(root: PathBuf) -> Self {
        WorkspacePaths { root }
    }

    pub fn data_dir(&self) -> PathBuf {
        self.root.join("data")
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.data_dir().join("sessions")
    }

    pub fn agents_dir(&self) -> PathBuf {
        self.data_dir().join("agents")
    }

    pub fn skills_dir(&self) -> PathBuf {
        self.root.join("skills")
    }

    pub fn bundled_skills_dir(&self) -> PathBuf {
        self.skills_dir().join("bundled")
    }

    pub fn user_skills_dir(&self) -> PathBuf {
        self.skills_dir().join("user")
    }

    pub fn settings_file(&self) -> PathBuf {
        self.data_dir().join("settings.json")
    }

    pub fn sessions_db(&self) -> PathBuf {
        self.sessions_dir().join("sessions.json")
    }

    pub fn session_dir(&self, session_id: &str) -> PathBuf {
        self.sessions_dir().join(session_id)
    }

    pub fn session_messages_file(&self, session_id: &str) -> PathBuf {
        self.session_dir(session_id).join("messages.jsonl")
    }

    /// Ensure the base directory skeleton exists (creating directories only,
    /// never copying seed files — that is done by bootstrap code).
    pub fn ensure(&self) -> Result<()> {
        for dir in [
            self.data_dir(),
            self.agents_dir(),
            self.sessions_dir(),
            self.bundled_skills_dir(),
            self.user_skills_dir(),
        ] {
            std::fs::create_dir_all(&dir)?;
        }
        Ok(())
    }

    /// Canonicalize a path for comparison (absolute + resolved symlinks).
    pub fn canonicalize_path(p: &Path) -> PathBuf {
        p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
    }
}
