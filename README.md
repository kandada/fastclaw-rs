> Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
> Licensed under GNU GPLv3, see LICENSE file for full license terms.

# fastclaw

An embeddable client-side AI agent library for desktop & mobile apps, plus a developer TUI.

`fastclaw` is the Rust-native implementation of [fastclaw](https://github.com/kandada/fastclaw) (Python),
built on top of [fastmind](https://github.com/kandada/fastmind-rs). It inherits the Python version's
ReAct loop, the two built-in tools (`run_shell` / `run_skills`), the skills mechanism and command safety,
and adds first-class **working-directory (WorkDir)** scoping. It is positioned as an agent kernel for
**TUI / Tauri / iOS / Android / HarmonyOS** — it ships **no HTTP/WebSocket server** of its own.

## Features

- **ReAct loop**: `agent ⇄ tools` with streaming output (`stream.chunk` / `stream.thinking` / `stream.tool_result`).
- **Dual LLM gateway**: `openai-client-rs` (DeepSeek / Kimi / MiniMax / Ollama and other OpenAI-compatible
  endpoints) and `anthropic-client-rs` (Claude / MiniMax / DeepSeek-anthropic), behind a unified
  `LlmProvider` abstraction.
- **Multimodal**: image input by URL or base64 on both gateways (the Anthropic gateway requires base64).
- **Thinking extraction**: the OpenAI gateway unifies `reasoning_content` / `reasoning` / `thinking` fields
  and `<think>` tags; the Anthropic gateway defaults to `adaptive` thinking (with enabled/none fallback)
  and round-trips `thinking_signature` across turns.
- **Two built-in tools**: `run_shell` (DENY / ASK / `CONFIRM:` safety gate + timeout + truncation) and
  `run_skills` (list / inspect / execute).
- **Skills**: meta-skills (`SKILL.md` instruction documents) + native Rust skills (`Skill` trait). The
  bundled `skill_creator` preset skill is seeded automatically into new workspaces.
- **Working directory (WorkDir)**: register / bind / default three-tier resolution; `run_shell`'s `cwd`
  and permission checks are constrained to the resolved directory set.
- **Data-format compatible**: `sessions.json` / `messages.jsonl` / `metadata.json` / `SKILL.md` match the
  Python version — existing workspaces can be migrated or shared in place.
- **FFI friendly**: no generics or lifetimes in the public API, streaming via callbacks, stable error codes.

## Quick start

```rust
use fastclaw::{FastClaw, FastClawConfig};

#[tokio::main]
async fn main() -> fastclaw::Result<()> {
    let claw = FastClaw::builder(FastClawConfig::default()).build();
    claw.start().await?;

    let session = claw.new_session(None);
    let reply = claw
        .run_chat(
            &session,
            "List the files in the current directory",
            Some(std::sync::Arc::new(|delta| print!("{delta}"))),
            None,
        )
        .await;
    println!("\n[reply {} chars]", reply.len());

    claw.stop().await?;
    Ok(())
}
```

Configure the LLM (`.env` or environment variables):

```
# OpenAI-compatible (DeepSeek, etc.)
LLM_API_KEY=...
LLM_API_URL=https://api.deepseek.com/v1
LLM_MODEL_NAME=deepseek-chat

# Anthropic
ANTHROPIC_API_KEY=...
ANTHROPIC_MODEL=claude-sonnet-4-20250514
```

The Anthropic gateway enables `adaptive` thinking by default; tune it in the agent's
`metadata.json` `llm` block:

```json
{ "gateway": "anthropic",
  "enable_thinking": false,               // explicitly disable thinking
  "thinking_budget_tokens": 30000 }       // budget used by the enabled fallback
```

Multimodal (image input):

```rust
use fastclaw::ImagePart;

let img = ImagePart::from_url("https://example.com/photo.png");   // OpenAI gateway
let img = ImagePart::from_base64("image/png", "...base64...");    // Anthropic gateway requires base64

claw.chat_with_images(&session, "What is in the picture?", &[img]).await?;
```

## Skills

Two kinds of skills:

| Kind | Description | Execution |
|------|-------------|-----------|
| **Meta** | a directory with `SKILL.md` (+ optional resources) | returns the instruction document; the agent follows it with `run_shell` |
| **Native** | a Rust struct implementing the `Skill` trait | runs directly |

```rust
claw.list_skills();                                  // discover
// The `run_skills` tool: "__list__", "__info__", or execute by name.
```

The bundled `skill_creator` meta-skill (`workspace_seed/skills/bundled/skill_creator/SKILL.md`) guides the
agent on how to create / update / optimize skills at runtime. Seed files are embedded at compile time and
written to the workspace on first start with file-level no-overwrite protection. See
[docs/skills.md](docs/skills.md).

## Workspace & working directories

| Concept | Meaning |
|---------|---------|
| **Workspace root** | fastclaw's metadata area (sessions / skills / agents / settings). Desktop default `~/.fastclaw-rs/workspace`; `FASTCLAW_WORKSPACE` overrides; mobile hosts inject a sandbox dir |
| **Working directory** | where a task reads / writes files. Register + per-session bind + default, resolved in three tiers |

```rust
claw.register_work_dir(Path::new("/path/to/project"))?;   // add to the trusted set
claw.bind_work_dir(&session, Path::new("/path/to/task"))?; // bind one task's directory
```

Bindings and defaults are persisted (`sessions.json` / `settings.json`) and restored on restart. See
[docs/workdir.md](docs/workdir.md) and the design doc `fastclaw-rs_design.md`.

## TUI

```bash
cargo run --bin fastclaw-tui --features tui
```

Slash commands: `/new` `/clear` `/session <id>` `/session_list` `/agent <id>` `/skill list`
`/workdir <path>` `/quit`. See [docs/tui.md](docs/tui.md).

## Directory structure

```
src/
├── api.rs          FastClaw / FastClawBuilder / FastClawConfig facade
├── agent/          ReAct loop, routing, SystemPrompt
├── llm/            LlmProvider abstraction + openai / anthropic adapters
├── tools/          run_shell / run_skills + command safety
├── skills/         skill registry + meta / native skills
├── workspace/      WorkspacePaths / WorkDirManager / SessionStore
├── session/        messages.jsonl persistence + context management
├── config.rs       Settings / AgentConfig loading
├── seed.rs         built-in workspace seeding (workspace_seed/)
└── error.rs        unified error type
```

## Testing

```bash
cargo test                        # unit + integration (mock LLM)
cargo test --all-features         # include TUI-feature builds
cargo clippy --all-targets
```

Real-LLM integration tests and a TUI end-to-end script (expect) are also included; they require a
workspace with real API keys. See `tests/llm_integration.rs` and `scripts/tui_e2e.exp`.

## Documentation

See `docs/` for architecture (`architecture.md`), skills (`skills.md`), working directories
(`workdir.md`), TUI usage (`tui.md`), embedding notes (`embedding.md`) and the Python→Rust migration
guide (`migration.md`).

## License

GPL-3.0-or-later (same as `fastmind` and the Python `fastclaw`). Its dependencies `openai-client-rs` /
`anthropic-client-rs` are Apache-2.0.
