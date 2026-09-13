> Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
> Licensed under GNU GPLv3, see LICENSE file for full license terms.

# fastclaw

面向桌面端 / 移动端通用 Agent 应用的可嵌入客户端库（+ 开发用 TUI）。

fastclaw 是 [fastclaw](https://github.com/kandada/fastclaw)（Python 版）的 Rust 原生实现，构建在
[fastmind](https://github.com/kandada/fastmind-rs) 之上。它延续 Python 版的 ReAct 循环、两个内置工具
（`run_shell` / `run_skills`）、skills 机制与命令安全，并新增一等公民的「工作目录（WorkDir）」约束，
定位为 **TUI / Tauri / iOS / Android / 鸿蒙** 的 agent 内核——**不内置任何 HTTP/WebSocket 服务**。

## 特性

- **ReAct 循环**：`agent ⇄ tools`，流式输出（`stream.chunk` / `stream.thinking` / `stream.tool_result`）。
- **双 LLM 网关**：`openai-client-rs`（DeepSeek / Kimi / MiniMax / Ollama 等 OpenAI 兼容）与
  `anthropic-client-rs`（Claude / MiniMax / DeepSeek-anthropic），统一 `LlmProvider` 抽象。
- **多模态**：支持图片输入（URL 或 base64），两条网关均可（Anthropic 网关需 base64）。
- **Thinking / 推理提取**：OpenAI 网关统一识别 `reasoning_content` / `reasoning` / `thinking`
  字段与 `<think>` 标签；Anthropic 网关默认 `adaptive` thinking（带 enabled/none 降级），
  捕获并多轮回传 `thinking_signature`。
- **两个内置工具**：`run_shell`（含 DENY / ASK / `CONFIRM:` 安全门 + 超时 + 截断）、
  `run_skills`（列表 / 查看 / 执行）。
- **System prompt 注入**：`set_injections` / `append_injection` / `injections` 让宿主运行时注入
  能力指南（如 fastbrowser 命令用法），每轮现读现拼、下一请求即生效，无需重绑会话。
- **Skills**：元技能（`SKILL.md` 指令文档）+ 原生 Rust 技能（`Skill` trait）；预置的
  `skill_creator` 技能会自动 seed 到新 workspace。
- **工作目录（WorkDir）**：注册 / 绑定 / 默认三级解析，`run_shell` 的 `cwd` 与安全判定均受约束。
- **数据格式兼容**：`sessions.json` / `messages.jsonl` / `metadata.json` / `SKILL.md` 与 Python 版一致，
  可迁移 / 共存。
- **FFI 友好**：公共 API 无泛型 / 生命周期，流式走回调，错误带稳定错误码。

## 快速开始

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
            "列出当前目录的文件",
            Some(std::sync::Arc::new(|delta| print!("{delta}"))),
            None,
        )
        .await;
    println!("\n[reply {} chars]", reply.len());

    claw.stop().await?;
    Ok(())
}
```

配置 LLM（`.env` 或环境变量）：

```
# OpenAI 兼容（DeepSeek 等）
LLM_API_KEY=...
LLM_API_URL=https://api.deepseek.com/v1
LLM_MODEL_NAME=deepseek-chat

# Anthropic
ANTHROPIC_API_KEY=...
ANTHROPIC_MODEL=claude-sonnet-4-20250514
```

Anthropic 网关默认启用 `adaptive` 思考；可在 agent `metadata.json` 的 `llm` 中调整：

```json
{ "gateway": "anthropic",
  "enable_thinking": false,               // 显式关闭思考
  "thinking_budget_tokens": 30000 }       // enabled 降级模式用到的 budget
```

多模态（图片输入）：

```rust
use fastclaw::ImagePart;

let img = ImagePart::from_url("https://example.com/photo.png");   // OpenAI 网关
let img = ImagePart::from_base64("image/png", "...base64...");    // Anthropic 网关要求 base64

claw.chat_with_images(&session, "图片里是什么？", &[img]).await?;
```

System prompt 注入（宿主能力感知，每轮生效）：

```rust
use fastclaw::PromptSection;

claw.set_injections(vec![PromptSection::new(
    "浏览器能力（fastbrowser）",
    "你有浏览器内核 fastbrowser，可用 run_shell 命令操作网页：\n- 感知：fastbrowser snapshot\n- 动作：fastbrowser click a / type e \"文字\"\n- 截图：fastbrowser screenshot <路径>\n完整命令表：fastbrowser --help",
)]);
claw.append_injection(PromptSection::new("安全提示", "不要运行 rm -rf /"));
let sections = claw.injections(); // Vec<PromptSection>
```

## Skills

两类技能：

| 类型 | 说明 | 执行方式 |
|------|------|----------|
| **元技能** | 一个含 `SKILL.md` 的目录（+ 可选资源文件） | 返回指令文档，agent 读后自行用 `run_shell` 完成步骤 |
| **原生技能** | 实现 `Skill` trait 的 Rust 代码 | 直接执行 |

```rust
claw.list_skills();                                  // 发现技能
// `run_skills` 工具：`"__list__"`、`"__info__"`，或按名执行
```

预置的 `skill_creator` 元技能（`workspace_seed/skills/bundled/skill_creator/SKILL.md`）指导 agent
如何在运行时创建 / 更新 / 优化技能。种子文件编译期内嵌进二进制，首次启动写入 workspace，
文件级保护不覆盖已存在文件。详见 [docs/skills.md](docs/skills.md)。

## Workspace 与工作目录

| 概念 | 说明 |
|------|------|
| **Workspace 根** | fastclaw 元数据区（会话 / skills / agents / settings）。桌面默认 `~/.fastclaw-rs/workspace`；`FASTCLAW_WORKSPACE` 可覆盖；移动端由宿主注入沙盒目录 |
| **工作目录** | 任务真正读写 / 产出文件的地方。注册 + 会话绑定 + 默认三级解析 |

```rust
claw.register_work_dir(Path::new("/path/to/project"))?;   // 加入信任目录集合
claw.bind_work_dir(&session, Path::new("/path/to/task"))?; // 绑定单个任务的目录
```

绑定与默认值都会持久化（`sessions.json` / `settings.json`），重启后自动恢复。详见
[docs/workdir.md](docs/workdir.md) 与设计文档 `fastclaw-rs_design.md`。

## TUI

```bash
cargo run --bin fastclaw-tui --features tui
```

斜杠命令：`/new` `/clear` `/session <id>` `/session_list` `/agent <id>` `/skill list`
`/workdir <path>` `/quit`。完整使用说明见 [docs/tui.md](docs/tui.md)。

## 目录结构

```
src/
├── api.rs          FastClaw / FastClawBuilder / FastClawConfig 门面
├── agent/          ReAct 循环、路由、SystemPrompt
├── llm/            LlmProvider 抽象 + openai / anthropic 适配
├── tools/          run_shell / run_skills + 命令安全
├── skills/         技能注册表 + 元技能 / 原生技能
├── workspace/      WorkspacePaths / WorkDirManager / SessionStore
├── session/        messages.jsonl 持久化 + 上下文管理
├── config.rs       Settings / AgentConfig 加载
├── seed.rs         内置 workspace 种子（workspace_seed/）
└── error.rs        统一错误
```

## 测试

```bash
cargo test                        # 单元 + 集成（mock LLM）
cargo test --all-features         # 含 TUI feature 构建
cargo clippy --all-targets
```

还包含真实 LLM 集成测试与 TUI 端到端脚本（expect），需要带真实 API key 的 workspace。
见 `tests/llm_integration.rs` 与 `scripts/tui_e2e.exp`。

## 文档

见 `docs/`：架构（`architecture.md`）、技能（`skills.md`）、工作目录（`workdir.md`）、
TUI（`tui.md`）、嵌入说明（`embedding.md`）、Python→Rust 迁移（`migration.md`）。

## License

GPL-3.0-or-later（与 fastmind / Python fastclaw 一致）。依赖 `openai-client-rs` /
`anthropic-client-rs` 为 Apache-2.0。
