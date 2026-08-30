// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Command permission / safety checks.

use std::path::PathBuf;

use fastclaw::tools::security::{
    check_command_permission, extract_paths_from_command, is_confirm_command, strip_confirm,
    strip_heredocs, Permission,
};

fn workdirs() -> Vec<PathBuf> {
    vec![PathBuf::from("/Users/me/projects/app")]
}

#[test]
fn deny_rm_rf_root() {
    let r = check_command_permission("rm -rf /", &workdirs());
    assert_eq!(r.permission, Permission::Deny);
}

#[test]
fn deny_mkfs() {
    let r = check_command_permission("mkfs.ext4 /dev/sda1", &workdirs());
    assert_eq!(r.permission, Permission::Deny);
}

#[test]
fn ask_shutdown() {
    let r = check_command_permission("shutdown -h now", &workdirs());
    assert_eq!(r.permission, Permission::AskUser);
}

#[test]
fn ask_fork_bomb() {
    let r = check_command_permission(":(){:|:&};:", &workdirs());
    assert_eq!(r.permission, Permission::AskUser);
}

#[test]
fn ask_system_path() {
    let r = check_command_permission("cat /etc/passwd", &workdirs());
    assert_eq!(r.permission, Permission::AskUser);
}

#[test]
fn allow_workdir_relative() {
    let r = check_command_permission("ls -la", &workdirs());
    assert_eq!(r.permission, Permission::Allow);
}

#[test]
fn allow_workdir_absolute() {
    let r = check_command_permission("cat /Users/me/projects/app/README.md", &workdirs());
    assert_eq!(r.permission, Permission::Allow);
}

#[test]
fn allow_harmless_device() {
    let r = check_command_permission("echo hi > /dev/null", &workdirs());
    assert_eq!(r.permission, Permission::Allow);
}

#[test]
fn confirm_detection_and_strip() {
    assert!(is_confirm_command("CONFIRM: rm -rf *"));
    assert_eq!(strip_confirm("CONFIRM: rm -rf *"), "rm -rf *");
    assert!(!is_confirm_command("rm -rf *"));
}

#[test]
fn heredoc_stripped() {
    let cmd = "cat <<EOF\nrm -rf /\nEOF";
    let stripped = strip_heredocs(cmd);
    assert!(!stripped.contains("rm -rf /"));
}

#[test]
fn extract_paths() {
    let paths = extract_paths_from_command("cat ./src/main.rs /tmp/log.txt");
    assert!(paths.iter().any(|p| p.ends_with("main.rs")));
    assert!(paths.iter().any(|p| p.contains("/tmp/log.txt")));
}

#[test]
fn deny_dd_write_device() {
    let r = check_command_permission("dd if=/dev/zero of=/dev/sda bs=1M", &workdirs());
    assert_eq!(r.permission, Permission::Deny);
}

#[test]
fn deny_kill_init() {
    let r = check_command_permission("kill -9 1", &workdirs());
    assert_eq!(r.permission, Permission::Deny);
}

#[test]
fn deny_chattr_passwd() {
    let r = check_command_permission("chattr -i /etc/passwd", &workdirs());
    assert_eq!(r.permission, Permission::Deny);
}

#[test]
fn ask_rm_rf_star() {
    let r = check_command_permission("rm -rf *", &workdirs());
    assert_eq!(r.permission, Permission::AskUser);
}

#[test]
fn ask_rm_rf_dot() {
    let r = check_command_permission("rm -rf .", &workdirs());
    assert_eq!(r.permission, Permission::AskUser);
}

#[test]
fn ask_reboot() {
    let r = check_command_permission("sudo reboot", &workdirs());
    assert_eq!(r.permission, Permission::AskUser);
}

#[test]
fn deny_veracrypt() {
    let r = check_command_permission("veracrypt -d /dev/sdb1", &workdirs());
    assert_eq!(r.permission, Permission::Deny);
}

#[test]
fn extract_paths_ignores_flags() {
    let paths = extract_paths_from_command("ls -la --color=auto");
    assert!(!paths.iter().any(|p| p.starts_with("--")));
}

#[test]
fn strip_heredoc_multiline_keeps_command() {
    let cmd = "sh -c 'cat <<EOF\nline one\nrm -rf /\nEOF\n'";
    let stripped = strip_heredocs(cmd);
    assert!(!stripped.contains("rm -rf /"));
}

#[test]
fn allow_plain_command_no_paths() {
    let r = check_command_permission("git status", &workdirs());
    assert_eq!(r.permission, Permission::Allow);
}
