// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! The `run_shell` tool: execute a shell command with permission checks,
//! timeout, output truncation, and working-directory scoping.

use std::path::PathBuf;
use std::process::Stdio;

use serde_json::{json, Value};

use crate::error::Result;
use crate::tools::security::{
    check_command_permission, is_confirm_command, strip_confirm, Permission,
};
use crate::tools::ToolContext;
use crate::State;

/// Execute a shell command (the body of the `run_shell` tool).
pub async fn run_shell(
    ctx: &ToolContext,
    session_id: &str,
    agent_work_dirs: &[PathBuf],
    state: &mut State,
    command: String,
    max_length: Option<i64>,
    timeout: Option<u64>,
) -> Result<String> {
    let mut command = command;
    let mut confirmed = false;
    if is_confirm_command(&command) {
        command = strip_confirm(&command);
        confirmed = true;
    }

    let allowed_dirs = ctx.workdirs.allowed(agent_work_dirs);
    let permission = check_command_permission(&command, &allowed_dirs);

    if permission.permission == Permission::AskUser && confirmed {
        return run_command(
            ctx,
            command,
            max_length,
            timeout,
            ctx.workdirs.resolve(session_id, agent_work_dirs),
        )
        .await;
    }

    match permission.permission {
        Permission::Deny => Ok(format!("Permission denied: {}", permission.reason)),
        Permission::AskUser => {
            // Track attempted commands to avoid an infinite ask loop.
            let already = {
                let list = state
                    .get("_ask_user_commands")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                list.iter().any(|v| v.as_str() == Some(command.as_str()))
            };
            if already {
                return Ok(format!(
                    "AskUser: {} (You already attempted this command. Do NOT repeat it without the CONFIRM: prefix.)",
                    permission.reason
                ));
            }
            let mut list = state
                .get("_ask_user_commands")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            list.push(json!(command));
            state.set("_ask_user_commands", Value::Array(list));
            Ok(format!("AskUser: {}", permission.reason))
        }
        Permission::Allow => {
            run_command(
                ctx,
                command,
                max_length,
                timeout,
                ctx.workdirs.resolve(session_id, agent_work_dirs),
            )
            .await
        }
    }
}

async fn run_command(
    ctx: &ToolContext,
    command: String,
    max_length: Option<i64>,
    timeout: Option<u64>,
    cwd: PathBuf,
) -> Result<String> {
    let settings = ctx.settings();
    let default_timeout = settings.run_shell_timeout.max(1);
    let effective_timeout = match timeout {
        Some(t) if t > 0 => t,
        _ => default_timeout,
    };

    let mut cmd = build_shell(&command);
    // Ensure the working directory exists (the shell would fail otherwise).
    let _ = std::fs::create_dir_all(&cwd);
    cmd.current_dir(&cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Ok(format!("Error: {e}")),
    };

    let output = match tokio::time::timeout(
        std::time::Duration::from_secs(effective_timeout),
        child.wait_with_output(),
    )
    .await
    {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Ok(format!("Error: {e}")),
        Err(_) => return Ok(format!("Error: Command timed out ({effective_timeout}s)")),
    };

    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    let err = String::from_utf8_lossy(&output.stderr).into_owned();
    if text.is_empty() && !err.is_empty() {
        text = err;
    }
    if text.trim().is_empty() {
        return Ok("(command completed, no output)".to_string());
    }

    let limit = max_length.unwrap_or(8000);
    if limit != -1 && text.chars().count() > limit as usize {
        let head: String = text.chars().take(limit as usize).collect();
        text = format!(
            "{head}\n...(truncated, {} total chars)",
            text.chars().count()
        );
    }
    Ok(text)
}

fn build_shell(command: &str) -> tokio::process::Command {
    #[cfg(unix)]
    {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd
    }
    #[cfg(windows)]
    {
        let mut cmd = tokio::process::Command::new("cmd");
        cmd.arg("/C").arg(command);
        cmd
    }
}
