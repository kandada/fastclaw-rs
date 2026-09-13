// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! FastClaw system prompt template.

use serde::{Deserialize, Serialize};

/// A host-injectable section appended to the system prompt, after the
/// per-agent personality (`SOUL.md` / `USER.md` / `AGENT.md`).
///
/// Unlike personality files (which are bound once per session and cached),
/// injections are held in memory on the `FastClaw` instance, re-read on every
/// LLM round, and take effect from the next request — no session rebind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptSection {
    pub title: String,
    pub content: String,
}

impl PromptSection {
    pub fn new(title: impl Into<String>, content: impl Into<String>) -> Self {
        PromptSection {
            title: title.into(),
            content: content.into(),
        }
    }
}

/// The system prompt template. Placeholders:
/// `{skills_list}`, `{session_id}`, `{extra_workspaces}`, `{workspace_path}`,
/// `{workdir}`.
pub const SYSTEM_PROMPT: &str = r#"You are an autonomous intelligent assistant, codename FastClaw.

## Core Capabilities
- You have the run_shell atomic capability to accomplish any task via shell commands
- You have the run_skills tool to execute predefined skills

## Working Mode (Important)

**In a single response, you can both reply to the user AND invoke tool instructions.**

1. **Talk while doing**: Reply to the user in text while executing tools
2. **Think then act**: First output thoughts (e.g. "Let me check..."), then invoke tools
3. **Reply only**: If the question can be answered directly, just output text
4. **Act only**: If only tool execution is needed, invoke the tool

Note: Tool calls are handled via the API's tool_calls mechanism, NOT by outputting JSON text. When a tool needs to be called, the system automatically executes it and returns the result. Just explain in natural language what you want to do — **do not** output any JSON-formatted tool call information in your response. Wait for tool results before responding.

## Tool Usage Examples

### run_shell (Shell command execution)
- "List current directory files" -> run_shell("ls -la")
- "Read file content" -> run_shell("cat filename.txt", max_length=500)  # default max_length is 8000
- "Search for a function in code" -> run_shell("rg 'function_name' .")
- "Create directory" -> run_shell("mkdir -p dir/path")
- "Network request" -> run_shell("curl -s --max-time 10 https://api.example.com/data")
- "Long running task" -> run_shell("pip install pandas", timeout=120)  # default timeout is 60s

Note: When editing files, try to work from the existing file content and prefer line-level or character-level edits over full rewrites.

### run_skills (Skill execution)
1. **List skills**: run_skills("__list__")
2. **View skill details (inspect only, does not run)**: run_skills("__info__", {"skill_name": "current_time"})
3. **Execute a skill**: run_skills("current_time")

Skills come in two kinds:
- **Script skills**: executing runs the script and returns its result.
- **Document skills** (SKILL.md only): executing returns the instruction document. When you receive such a document, DO NOT simply relay it to the user — read it, then carry out its steps yourself using run_shell, reading any listed resource files on demand.

## Available Skills
{skills_list}

## Graph Flow Control
- **Has tool_calls**: tool_node executes tool -> result returns to agent -> you continue replying
- **No tool_calls**: after replying to user, flow ends (task complete)

## Working Directory
Your current working directory is: {workdir}
All run_shell commands run from this directory; use relative paths to keep produced files here.

Full read/write access to these directories:
- {workdir}
- {extra_workspaces}

## Context Management
Your workspace is located at: {workspace_path}
Your conversation context is stored in the session directory:
- Path format: workspace/data/sessions/{session_id}/messages.jsonl
- Current session ID: {session_id}

When context approaches the threshold, the AI automatically unloads older messages. To read previous content, use:
run_shell("cat workspace/data/sessions/{session_id}/messages.jsonl")

## Directory / File Permissions
### Full read/write access:
- Working directories listed above
### Restricted access:
- System directories like /etc/, /usr/, etc.
- Even when restricted, you may attempt if the task requires it

### Sensitive Operation Confirmation
Execute necessary commands promptly without asking the user unnecessarily. When run_shell returns "AskUser: ..." or prompts for "CONFIRM: <command>":
- Explain that the operation needs user confirmation, then ask the user
- If the user agrees, immediately call run_shell("CONFIRM: <command>")
- Note: CONFIRM: goes in the run_shell parameter, not in the chat message

## Language
- Adapt your language based on the user's message. If the user writes in Chinese, respond in Chinese. If in English, respond in English.
- When unsure which language to use, prefer English.
"#;

/// Format the system prompt with the given dynamic values.
///
/// Layout order: template → personality → host injections. Each injection with
/// non-empty content is appended as `## {title}` + content (or bare content
/// when the title is empty).
pub fn format_system_prompt(
    skills_list: &str,
    session_id: &str,
    personality: &str,
    work_dirs: &[String],
    workspace_path: &str,
    workdir: &str,
    injections: &[PromptSection],
) -> String {
    let extra = if work_dirs.is_empty() {
        "(not configured)".to_string()
    } else {
        work_dirs.join(", ")
    };
    let mut prompt = SYSTEM_PROMPT
        .replace("{skills_list}", skills_list)
        .replace("{session_id}", session_id)
        .replace("{extra_workspaces}", &extra)
        .replace("{workspace_path}", workspace_path)
        .replace("{workdir}", workdir);
    if !personality.is_empty() {
        prompt.push_str(&format!("\n\n{personality}"));
    }
    for sec in injections {
        let content = sec.content.trim();
        if content.is_empty() {
            continue;
        }
        let title = sec.title.trim();
        if title.is_empty() {
            prompt.push_str(&format!("\n\n{content}"));
        } else {
            prompt.push_str(&format!("\n\n## {title}\n{content}"));
        }
    }
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injections_appended_after_personality() {
        let prompt = format_system_prompt(
            "- a: skill",
            "s1",
            "## SOUL\nbe nice",
            &[],
            "/ws",
            "/work",
            &[PromptSection::new(
                "浏览器能力（fastbrowser）",
                "你有浏览器内核 fastbrowser。",
            )],
        );
        assert!(prompt.contains("## SOUL\nbe nice"));
        assert!(prompt.contains("\n\n## 浏览器能力（fastbrowser）\n你有浏览器内核 fastbrowser。"));
        assert!(prompt.ends_with("你有浏览器内核 fastbrowser。"));
    }

    #[test]
    fn empty_injections_are_skipped() {
        let prompt = format_system_prompt("- a: skill", "s1", "", &[], "/ws", "/work", &[]);
        // Nothing appended after the base template when there are no injections.
        assert!(prompt
            .trim_end()
            .ends_with("- When unsure which language to use, prefer English."));
    }

    #[test]
    fn empty_title_renders_bare_content() {
        let prompt = format_system_prompt(
            "- a: skill",
            "s1",
            "",
            &[],
            "/ws",
            "/work",
            &[PromptSection::new("", "just a note")],
        );
        assert!(prompt.ends_with("just a note"));
    }

    #[test]
    fn multiple_injections_render_in_order() {
        let prompt = format_system_prompt(
            "- a: skill",
            "s1",
            "",
            &[],
            "/ws",
            "/work",
            &[
                PromptSection::new("t1", "c1"),
                PromptSection::new("t2", "c2"),
            ],
        );
        let i1 = prompt.find("## t1\nc1").expect("t1");
        let i2 = prompt.find("## t2\nc2").expect("t2");
        assert!(i1 < i2, "sections must keep insertion order");
    }

    #[test]
    fn duplicate_titles_render_twice() {
        let prompt = format_system_prompt(
            "- a: skill",
            "s1",
            "",
            &[],
            "/ws",
            "/work",
            &[PromptSection::new("dup", "first"), PromptSection::new("dup", "second")],
        );
        assert_eq!(prompt.matches("## dup").count(), 2);
    }

    #[test]
    fn title_and_content_are_trimmed() {
        let prompt = format_system_prompt(
            "- a: skill",
            "s1",
            "",
            &[],
            "/ws",
            "/work",
            &[PromptSection::new("  spaced  ", "\n body \n")],
        );
        assert!(prompt.contains("## spaced\nbody"));
    }

    #[test]
    fn whitespace_title_renders_bare_content() {
        let prompt = format_system_prompt(
            "- a: skill",
            "s1",
            "",
            &[],
            "/ws",
            "/work",
            &[PromptSection::new("   ", "bare body")],
        );
        assert!(prompt.ends_with("\n\nbare body"));
    }

    #[test]
    fn injection_content_is_not_placeholder_substituted() {
        // The template substitution pass happens before injections are appended,
        // so placeholder-looking text in an injection survives verbatim.
        let prompt = format_system_prompt(
            "- a: skill",
            "s1",
            "",
            &[],
            "/ws",
            "/work",
            &[PromptSection::new(
                "literal",
                "keep {session_id} {workdir} {skills_list}",
            )],
        );
        assert!(prompt.contains("keep {session_id} {workdir} {skills_list}"));
    }

    #[test]
    fn injections_apply_without_personality() {
        let prompt = format_system_prompt(
            "- a: skill",
            "s1",
            "",
            &[],
            "/ws",
            "/work",
            &[PromptSection::new("t", "c")],
        );
        assert!(prompt.ends_with("## t\nc"));
    }

    #[test]
    fn large_unicode_content_is_preserved() {
        let content = "能力：浏览器 🦞".repeat(2000);
        let prompt = format_system_prompt(
            "- a: skill",
            "s1",
            "",
            &[],
            "/ws",
            "/work",
            &[PromptSection::new("big", content.clone())],
        );
        assert!(prompt.contains(&content));
    }
}
