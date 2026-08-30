# Python fastclaw → fastclaw-rs 迁移对照

## 概念对照

| Python fastclaw | fastclaw-rs |
|-----------------|-------------|
| `fastclaw/core/app.py`（`FastMind` app + `fastclaw_agent` + `graph`） | `src/agent/` + `src/api.rs`（`FastClaw`） |
| `openai.AsyncOpenAI`（仅 OpenAI 兼容） | `llm/LlmProvider` 双网关（openai + anthropic） |
| `@app.tool run_shell` / `run_skills` | `src/tools/`（`ToolContext::execute` 派发） |
| `check_command_permission` + `DENY/ASK_PATTERNS` | `src/tools/security.rs` |
| skills `main.py` 脚本技能 + `SKILL.md` 文档技能 | 元技能（`SKILL.md`）+ 原生 `Skill` trait（`main.py` 不再执行） |
| `get_workspace_path()` / `extra_workspaces` | `WorkspacePaths` / `WorkDirManager`（三级解析） |
| `save/load_messages_from_jsonl` / `SessionStore` | `src/session/` / `src/workspace/session.rs`（格式兼容） |
| FastAPI + WebUI + SSE/WS + channels + cron | **不做**（客户端定位） |

## 数据格式兼容

`sessions.json`、`messages.jsonl`、`metadata.json`、`SOUL.md`/`USER.md`/`AGENT.md`、`SKILL.md`
文件与格式与 Python 版一致，可直接迁移。唯一例外：`skills/*/main.py` 脚本技能在 Rust 不可执行，
v1 只保留其 `SKILL.md` 作元技能。

## 桌面端共存

| 维度 | Python | fastclaw-rs |
|------|--------|-------------|
| 默认 workspace 根 | `~/.fastclaw/workspace` | `~/.fastclaw-rs/workspace` |
| 迁移/共享 | — | `FASTCLAW_WORKSPACE` 指向旧目录即「原地接管」 |
| 并发保护 | PID 文件 | `data/.lock`（`fs2`） |

## 移动端差异

- Python 版是服务（无沙盒概念）；fastclaw-rs 在移动端运行于应用沙盒，workspace 根由宿主注入。
- `run_shell` 在 iOS 不可用 → 用 `extra_tools` 注入设备能力工具替代。
