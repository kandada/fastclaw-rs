// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Workspace path resolution and skeleton creation.

use std::path::PathBuf;

use fastclaw::WorkspacePaths;

#[test]
fn injected_root_wins() {
    let p = WorkspacePaths::resolve(Some(PathBuf::from("/custom/root")));
    assert_eq!(p, PathBuf::from("/custom/root"));
}

#[test]
fn subpath_accessors() {
    let p = WorkspacePaths::new(PathBuf::from("/root"));
    assert_eq!(p.root, PathBuf::from("/root"));
    assert_eq!(p.data_dir(), PathBuf::from("/root/data"));
    assert_eq!(p.agents_dir(), PathBuf::from("/root/data/agents"));
    assert_eq!(p.sessions_dir(), PathBuf::from("/root/data/sessions"));
    assert_eq!(p.settings_file(), PathBuf::from("/root/data/settings.json"));
    assert_eq!(
        p.sessions_db(),
        PathBuf::from("/root/data/sessions/sessions.json")
    );
    assert_eq!(p.skills_dir(), PathBuf::from("/root/skills"));
    assert_eq!(
        p.bundled_skills_dir(),
        PathBuf::from("/root/skills/bundled")
    );
    assert_eq!(p.user_skills_dir(), PathBuf::from("/root/skills/user"));
    assert_eq!(p.session_dir("s1"), PathBuf::from("/root/data/sessions/s1"));
    assert_eq!(
        p.session_messages_file("s1"),
        PathBuf::from("/root/data/sessions/s1/messages.jsonl")
    );
}

#[test]
fn ensure_creates_skeleton() {
    let dir = tempfile::tempdir().unwrap();
    let p = WorkspacePaths::new(dir.path().join("ws"));
    p.ensure().unwrap();
    assert!(p.data_dir().exists());
    assert!(p.agents_dir().exists());
    assert!(p.sessions_dir().exists());
    assert!(p.bundled_skills_dir().exists());
    assert!(p.user_skills_dir().exists());
}

#[test]
fn canonicalize_absolute_path() {
    let p = WorkspacePaths::canonicalize_path(std::path::Path::new("/usr/../usr/local"));
    assert_eq!(p, PathBuf::from("/usr/local"));
}

#[test]
fn canonicalize_missing_falls_back_to_input() {
    let input = PathBuf::from("/definitely/does/not/exist/path");
    let p = WorkspacePaths::canonicalize_path(&input);
    assert_eq!(p, input);
}
