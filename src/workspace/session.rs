// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Thread-safe session registry backed by `sessions.json`.

use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{json, Value};

/// A session registry persisted as a JSON object (`session_id -> {..}`).
pub struct SessionStore {
    db_file: PathBuf,
    lock: Mutex<()>,
}

impl SessionStore {
    pub fn new(db_file: PathBuf) -> Self {
        SessionStore {
            db_file,
            lock: Mutex::new(()),
        }
    }

    pub fn ensure_db(&self) {
        if let Some(parent) = self.db_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if !self.db_file.exists() {
            let _ = std::fs::write(&self.db_file, "{}");
        }
    }

    pub fn load(&self) -> Value {
        let _guard = self.lock.lock().unwrap();
        self.ensure_db();
        match std::fs::read_to_string(&self.db_file) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|_| json!({})),
            Err(_) => json!({}),
        }
    }

    pub fn save(&self, sessions: &Value) {
        let _guard = self.lock.lock().unwrap();
        self.ensure_db();
        let s = serde_json::to_string_pretty(sessions).unwrap_or_else(|_| "{}".to_string());
        let _ = std::fs::write(&self.db_file, s);
    }
}
