// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Working-directory management.

use std::path::PathBuf;

use fastclaw::WorkDirManager;

mod common;
use common::build_claw;

#[test]
fn resolve_three_tier_priority() {
    let m = WorkDirManager::new(PathBuf::from("/default"));
    let agent_dirs = vec![PathBuf::from("/agent")];

    // No binding → first agent dir.
    assert_eq!(m.resolve("s1", &agent_dirs), PathBuf::from("/agent"));

    // No binding, empty agent dirs → default.
    assert_eq!(m.resolve("s1", &[]), PathBuf::from("/default"));

    // Session binding wins over everything.
    m.bind("s1", &PathBuf::from("/bound")).unwrap();
    assert_eq!(m.resolve("s1", &agent_dirs), PathBuf::from("/bound"));

    m.unbind("s1");
    assert_eq!(m.resolve("s1", &agent_dirs), PathBuf::from("/agent"));
}

#[test]
fn register_dedups() {
    let m = WorkDirManager::new(PathBuf::from("/default"));
    m.register(&PathBuf::from("/a")).unwrap();
    m.register(&PathBuf::from("/a")).unwrap();
    m.register(&PathBuf::from("/b")).unwrap();
    assert_eq!(m.list().len(), 2);
    m.unregister(&PathBuf::from("/a")).unwrap();
    assert_eq!(m.list(), vec![PathBuf::from("/b")]);
}

#[test]
fn allowed_includes_default_registered_and_agent() {
    let m = WorkDirManager::new(PathBuf::from("/default"));
    m.register(&PathBuf::from("/reg")).unwrap();
    let allowed = m.allowed(&[PathBuf::from("/agent")]);
    assert!(allowed.contains(&PathBuf::from("/default")));
    assert!(allowed.contains(&PathBuf::from("/reg")));
    assert!(allowed.contains(&PathBuf::from("/agent")));
}

fn build_on(root: &std::path::Path) -> (fastclaw::FastClaw, std::sync::Arc<common::MockProvider>) {
    build_claw(root, vec![])
}

#[test]
fn register_work_dir_persists_to_settings_and_survives_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    {
        let (claw, _) = build_on(&root);
        claw.register_work_dir(&PathBuf::from("/reg")).unwrap();
        assert!(claw.get_settings().work_dirs.contains(&PathBuf::from("/reg")));
    }
    let (claw2, _) = build_on(&root);
    assert!(claw2.list_work_dirs().contains(&PathBuf::from("/reg")));
}

#[test]
fn set_default_work_dir_persists_and_survives_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    {
        let (claw, _) = build_on(&root);
        claw.set_default_work_dir(&PathBuf::from("/proj")).unwrap();
        assert_eq!(claw.default_work_dir(), PathBuf::from("/proj"));
        assert_eq!(
            claw.get_settings().default_work_dir,
            Some(PathBuf::from("/proj"))
        );
    }
    let (claw2, _) = build_on(&root);
    assert_eq!(claw2.default_work_dir(), PathBuf::from("/proj"));
}

#[test]
fn bind_work_dir_persists_to_sessions_json_and_restores() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let sid;
    {
        let (claw, _) = build_on(&root);
        sid = claw.new_session(None);
        claw.bind_work_dir(&sid, &PathBuf::from("/task")).unwrap();
        assert_eq!(claw.resolve_work_dir(&sid, None), PathBuf::from("/task"));
    }
    let (claw2, _) = build_on(&root);
    assert_eq!(claw2.resolve_work_dir(&sid, None), PathBuf::from("/task"));
}

#[test]
fn unbind_work_dir_clears_persisted_binding() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let sid;
    {
        let (claw, _) = build_on(&root);
        sid = claw.new_session(None);
        claw.bind_work_dir(&sid, &PathBuf::from("/task")).unwrap();
        claw.unbind_work_dir(&sid);
    }
    let (claw2, _) = build_on(&root);
    // No binding restored → falls back to the default work dir.
    let default = claw2.default_work_dir();
    assert_eq!(claw2.resolve_work_dir(&sid, None), default);
}
