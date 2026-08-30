// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Working-directory management: a set of trusted directories plus a default,
//! with optional per-session bindings. Mirrors aacode-style "project directory"
//! scoping, and drives `run_shell`'s `cwd` + permission checks.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use crate::error::{Error, Result};

#[derive(Debug, Default)]
struct WorkDirState {
    /// Registered (trusted) working directories.
    dirs: Vec<PathBuf>,
    /// Default working directory when nothing else matches.
    default: PathBuf,
    /// Per-session bindings: session_id -> work dir.
    bindings: HashMap<String, PathBuf>,
}

/// Thread-safe working-directory manager.
#[derive(Debug, Default)]
pub struct WorkDirManager {
    inner: RwLock<WorkDirState>,
}

impl WorkDirManager {
    pub fn new(default: PathBuf) -> Self {
        WorkDirManager {
            inner: RwLock::new(WorkDirState {
                default,
                ..Default::default()
            }),
        }
    }

    fn normalize(dir: &Path) -> PathBuf {
        if dir.is_absolute() {
            dir.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(dir)
        }
    }

    /// Register a working directory into the trusted set.
    pub fn register(&self, dir: &Path) -> Result<()> {
        let dir = Self::normalize(dir);
        let mut st = self
            .inner
            .write()
            .map_err(|_| Error::WorkDir("lock poisoned".into()))?;
        if !st.dirs.iter().any(|d| d == &dir) {
            st.dirs.push(dir);
        }
        Ok(())
    }

    pub fn unregister(&self, dir: &Path) -> Result<()> {
        let dir = Self::normalize(dir);
        let mut st = self
            .inner
            .write()
            .map_err(|_| Error::WorkDir("lock poisoned".into()))?;
        st.dirs.retain(|d| d != &dir);
        Ok(())
    }

    pub fn list(&self) -> Vec<PathBuf> {
        self.inner
            .read()
            .map(|st| st.dirs.clone())
            .unwrap_or_default()
    }

    pub fn set_default(&self, dir: &Path) -> Result<()> {
        let dir = Self::normalize(dir);
        let mut st = self
            .inner
            .write()
            .map_err(|_| Error::WorkDir("lock poisoned".into()))?;
        st.default = dir;
        Ok(())
    }

    pub fn default(&self) -> PathBuf {
        self.inner
            .read()
            .map(|st| st.default.clone())
            .unwrap_or_else(|_| PathBuf::from("."))
    }

    pub fn bind(&self, session_id: &str, dir: &Path) -> Result<()> {
        let dir = Self::normalize(dir);
        let mut st = self
            .inner
            .write()
            .map_err(|_| Error::WorkDir("lock poisoned".into()))?;
        st.bindings.insert(session_id.to_string(), dir);
        Ok(())
    }

    pub fn unbind(&self, session_id: &str) {
        if let Ok(mut st) = self.inner.write() {
            st.bindings.remove(session_id);
        }
    }

    pub fn binding(&self, session_id: &str) -> Option<PathBuf> {
        self.inner
            .read()
            .ok()
            .and_then(|st| st.bindings.get(session_id).cloned())
    }

    /// Three-tier resolution: session binding → first agent work_dir → default.
    pub fn resolve(&self, session_id: &str, agent_work_dirs: &[PathBuf]) -> PathBuf {
        let st = self.inner.read().unwrap();
        if let Some(dir) = st.bindings.get(session_id) {
            return dir.clone();
        }
        if let Some(dir) = agent_work_dirs.first() {
            return dir.clone();
        }
        st.default.clone()
    }

    /// The full set of directories the shell tool may write to:
    /// default + registered + the given agent work dirs.
    pub fn allowed(&self, agent_work_dirs: &[PathBuf]) -> Vec<PathBuf> {
        let st = self.inner.read().unwrap();
        let mut out: Vec<PathBuf> = Vec::new();
        out.push(st.default.clone());
        out.extend(st.dirs.iter().cloned());
        out.extend(agent_work_dirs.iter().cloned());
        out
    }
}
