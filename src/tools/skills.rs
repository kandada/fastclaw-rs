// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! The `run_skills` tool: list / inspect / execute skills.

use serde_json::{json, Value};

use crate::error::Result;
use crate::tools::ToolContext;

/// Execute the `run_skills` tool.
pub async fn run_skills(
    ctx: &ToolContext,
    skill_name: Option<String>,
    params: Option<Value>,
    timeout: Option<u64>,
) -> Result<String> {
    let params = params.unwrap_or_else(|| json!({}));
    let name = skill_name.unwrap_or_default();

    match name.as_str() {
        "__list__" | "list" | "" => {
            let skills = ctx.skills.list();
            if skills.is_empty() {
                return Ok("No skills available".to_string());
            }
            let lines = skills
                .iter()
                .map(|s| format!("- {}: {}", s.name, s.description))
                .collect::<Vec<_>>();
            Ok(format!("Available skills:\n{}", lines.join("\n")))
        }
        "__info__" | "info" => {
            let target = params
                .get("skill_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if target.is_empty() {
                return Ok("Error: skill_name is required for __info__ mode".to_string());
            }
            let info = match ctx.skills.get(target) {
                Some(i) => i,
                None => return Ok(format!("Error: Skill '{target}' not found")),
            };
            if let Some(md) = crate::skills::registry::find_skill_md(&info.path) {
                if let Ok(content) = std::fs::read_to_string(&md) {
                    return Ok(content);
                }
            }
            Ok(format!(
                "Skill: {}\nDescription: {}",
                info.name, info.description
            ))
        }
        _ => {
            let default_timeout = ctx.settings().run_skills_timeout.max(1);
            let effective_timeout = match timeout {
                Some(t) if t > 0 => t,
                _ => default_timeout,
            };
            ctx.skills.execute(&name, params, effective_timeout).await
        }
    }
}
