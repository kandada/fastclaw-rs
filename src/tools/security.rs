// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Shell command safety: deny/ask patterns + path-based permission checks.

use std::path::PathBuf;
use std::sync::OnceLock;

use regex::Regex;

/// The outcome of a permission check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Permission {
    Allow,
    AskUser,
    Deny,
}

/// Result of checking a command.
#[derive(Debug, Clone)]
pub struct PermissionResult {
    pub permission: Permission,
    pub reason: String,
}

impl PermissionResult {
    fn allow() -> Self {
        PermissionResult {
            permission: Permission::Allow,
            reason: String::new(),
        }
    }
    fn ask(reason: impl Into<String>) -> Self {
        PermissionResult {
            permission: Permission::AskUser,
            reason: reason.into(),
        }
    }
    fn deny(reason: impl Into<String>) -> Self {
        PermissionResult {
            permission: Permission::Deny,
            reason: reason.into(),
        }
    }
}

/// The `CONFIRM:` prefix used to re-run a gated command after user approval.
pub const CONFIRM_PREFIX: &str = "CONFIRM:";

/// Commands that are always blocked (destructive / system-breaking).
static DENY_PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();

fn deny_patterns() -> &'static [(Regex, &'static str)] {
    DENY_PATTERNS.get_or_init(|| {
        let pats: Vec<(&str, &'static str)> = vec![
            (
                r"rm\s+-rf\s+/\s*$",
                "rm -rf / (recursive root deletion, destroys system)",
            ),
            (
                r"rm\s+-rf\s+/\s",
                "rm -rf / (recursive root deletion, destroys system)",
            ),
            (r"\bmkfs\b", "mkfs (format disk, destroys data)"),
            (r"\bmkfs\.", "mkfs.* (format disk, destroys data)"),
            (r"\bnewfs\b", "newfs (create filesystem, destroys data)"),
            (r"\bwipefs\b", "wipefs (erase filesystem header)"),
            (
                r"\bdd\s+.*of=/dev/",
                "dd write to device (may corrupt disk)",
            ),
            (
                r"\bdd\s+.*of=/dev/sd",
                "dd write to device (may corrupt disk)",
            ),
            (
                r"\bdd\s+.*of=/dev/nvme",
                "dd write to device (may corrupt disk)",
            ),
            (r"\bfdisk\b.*-d", "fdisk -d (delete partition)"),
            (r"\bparted\b.*rm", "parted rm (delete partition)"),
            (r"\bsfdisk\b.*-d", "sfdisk -d (delete partition)"),
            (
                r"\bcryptsetup\b.*lukserase",
                "cryptsetup luksErase (erase LUKS header)",
            ),
            (
                r"\bcryptsetup\b.*luksclose",
                "cryptsetup luksClose (close encrypted volume)",
            ),
            (
                r"\bveracrypt\b.*-d",
                "veracrypt -d (decrypt volume, loses data)",
            ),
            (r"\bkill\s+-9\s+1\b", "kill -9 1 (kill init process)"),
            (r"\bpkill\s+-9\s+-1\b", "pkill -9 -1 (kill all processes)"),
            (r"\bkillall\s+-9\b", "killall -9 (force kill all processes)"),
            (
                r"\bchattr\s+-i\b.*(passwd|shadow|group|gshadow)",
                "chattr -i lock system files",
            ),
            (
                r"\bchattr\s+-a\b.*(passwd|shadow|group|gshadow)",
                "chattr -a modify system files",
            ),
        ];
        pats.iter()
            .map(|(p, d)| (Regex::new(p).expect("valid deny regex"), *d))
            .collect()
    })
}

/// Commands that require explicit user confirmation (via `CONFIRM:`).
static ASK_PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();

fn ask_patterns() -> &'static [(Regex, &'static str)] {
    ASK_PATTERNS.get_or_init(|| {
        let pats: Vec<(&str, &'static str)> = vec![
            (
                r"rm\s+-rf\s+\*\s*$",
                "rm -rf * (recursive delete, may remove important files)",
            ),
            (r"rm\s+-rf\s+\.", "rm -rf . (delete current directory)"),
            (r":\(\)\{:\|:&\};:", "fork bomb (exhausts system resources)"),
            (r"\bshutdown\b", "shutdown (shut down system)"),
            (r"\breboot\b", "reboot (reboot system)"),
            (r"\bhalt\b", "halt (halt system)"),
            (r"\bpoweroff\b", "poweroff (power off system)"),
            (r"\binit\s+0\b", "init 0 (shut down system)"),
            (r"\binit\s+6\b", "init 6 (reboot system)"),
            (r">\s*/dev/sd", "redirect to disk device (may corrupt data)"),
            (
                r">\s*/dev/null\s*>&",
                "redirect to null and close output (may lose data)",
            ),
        ];
        pats.iter()
            .map(|(p, d)| (Regex::new(p).expect("valid ask regex"), *d))
            .collect()
    })
}

/// System directories that are restricted regardless of workdir.
const SYSTEM_CORE_PATHS: &[&str] = &[
    "/etc/", "/usr/", "/sbin/", "/bin/", "/sys/", "/proc/", "/dev/",
];

/// Devices that are harmless to touch even though they are under /dev/.
const HARMLESS_DEVICES: &[&str] = &[
    "/dev/null",
    "/dev/zero",
    "/dev/full",
    "/dev/urandom",
    "/dev/random",
];

/// Strip heredoc bodies, keeping only the command structure for safety checks.
pub fn strip_heredocs(text: &str) -> String {
    let marker = Regex::new(r"<<-?\s*'?([A-Za-z_][A-Za-z0-9_]*)'?").expect("heredoc marker regex");
    let mut out: Vec<String> = Vec::new();
    let mut heredoc: Option<String> = None;
    for line in text.lines() {
        if let Some(delim) = &heredoc {
            if line.trim() == delim.as_str() {
                heredoc = None;
            }
            continue;
        }
        if let Some(delim) = marker.captures_iter(line).last().map(|c| c[1].to_string()) {
            heredoc = Some(delim);
            let prefix = line.find("<<").map(|p| &line[..p]).unwrap_or(line);
            if !prefix.trim().is_empty() {
                out.push(prefix.trim_end().to_string());
            }
        } else {
            out.push(line.to_string());
        }
    }
    out.join("\n")
}

/// Extract candidate file paths from a command for path-based checks.
pub fn extract_paths_from_command(command: &str) -> Vec<String> {
    let re = Regex::new(r"^[\w\.\-\/~]").expect("valid path token regex");
    command
        .replace('\\', " ")
        .split_whitespace()
        .filter_map(|token| {
            let token = token.trim();
            if token.is_empty() || token.starts_with('-') {
                return None;
            }
            if !re.is_match(token) {
                return None;
            }
            if !(token.contains('/') || token.contains('~') || token.starts_with('.')) {
                return None;
            }
            let clean = token.trim_end_matches([',', ';', ':', '\'', '"']);
            if clean.is_empty()
                || clean.starts_with("&&")
                || clean.starts_with("||")
                || clean.starts_with('|')
                || clean.starts_with(';')
                || clean.starts_with('>')
                || clean.starts_with('<')
            {
                return None;
            }
            Some(clean.to_string())
        })
        .collect()
}

fn resolve(p: &str) -> PathBuf {
    let pb = PathBuf::from(p);
    pb.canonicalize().unwrap_or(pb)
}

/// Check a shell command against deny/ask patterns and path restrictions.
pub fn check_command_permission(command: &str, allowed_dirs: &[PathBuf]) -> PermissionResult {
    let lower = strip_heredocs(command).to_lowercase();

    for (pattern, desc) in deny_patterns() {
        if pattern.is_match(&lower) {
            return PermissionResult::deny(format!(
                "Dangerous command blocked ({desc}). Use a safer approach if needed."
            ));
        }
    }

    for (pattern, desc) in ask_patterns() {
        if pattern.is_match(&lower) {
            return PermissionResult::ask(format!(
                "Sensitive operation detected ({desc}). Ask the user for permission. If confirmed, re-run with: {CONFIRM_PREFIX} <command>"
            ));
        }
    }

    let command_paths = extract_paths_from_command(command);
    for path in &command_paths {
        let resolved = resolve(path);
        let path_str = resolved.to_string_lossy().into_owned();

        // Inside an allowed working directory → allow.
        let in_allowed = allowed_dirs.iter().any(|dir| {
            let dir = resolve(&dir.to_string_lossy());
            path_str.starts_with(dir.to_string_lossy().as_ref())
        });
        if in_allowed {
            continue;
        }

        // System core paths (with harmless-device exemption) → ask.
        for sys in SYSTEM_CORE_PATHS {
            if path_str.contains(sys) || path.contains(sys) {
                if HARMLESS_DEVICES
                    .iter()
                    .any(|hd| path_str == *hd || path_str.ends_with(hd))
                {
                    continue;
                }
                return PermissionResult::ask(
                    "This operation is restricted. Ask the user for permission. If confirmed, re-run with: CONFIRM: <command>",
                );
            }
        }
    }

    PermissionResult::allow()
}

/// Validate a permission decision helper for tests.
pub fn is_confirm_command(command: &str) -> bool {
    command.trim_start().starts_with(CONFIRM_PREFIX)
}

pub fn strip_confirm(command: &str) -> String {
    command
        .trim_start()
        .strip_prefix(CONFIRM_PREFIX)
        .unwrap_or(command)
        .trim()
        .to_string()
}
