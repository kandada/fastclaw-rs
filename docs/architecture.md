# 架构说明

fastclaw-rs 是一个构建在 fastmind-rs 之上的**应用层库**，采用单 crate 结构。

## 分层

```
┌───────────────────────────────────────────────────────────────┐
│ 宿主应用（TUI / Tauri / iOS / Android / 鸿蒙）                  │
├───────────────────────────────────────────────────────────────┤
│ api.rs        FastClaw / FastClawBuilder / FastClawConfig     │
├───────────────────────────────────────────────────────────────┤
│ agent/        ReAct 循环（agent ⇄ tools）、路由、SystemPrompt │
│ llm/          LlmProvider 抽象 + openai / anthropic 适配       │
│ tools/        run_shell / run_skills + 命令安全                │
│ skills/       技能注册表 + 元技能 / 原生技能                    │
│ workspace/    WorkspacePaths / WorkDirManager / SessionStore  │
│ session/      messages.jsonl + 上下文管理                     │
│ config.rs     Settings / AgentConfig                          │
│ error.rs      统一错误                                         │
├───────────────────────────────────────────────────────────────┤
│ fastmind-rs（框架）                                            │
├───────────────────────────────────────────────────────────────┤
│ openai-client-rs │ anthropic-client-rs                        │
└───────────────────────────────────────────────────────────────┘
```

## ReAct 循环

图结构（注册为 fastmind `Graph` 的 `main`）：

```
agent ──(route: 有 tool_calls)──▶ tools ──▶ agent
   └──(route: 无 tool_calls)──▶ __end__
```

- `agent` 节点：懒加载 agent 配置/人格/历史 → 处理上一轮 `tool_results`（回填 `role:"tool"` 消息）
  → 消息去重（`message_id` 优先）→ 上下文管理（unload/fix）→ 流式调用 LLM → 实时发
  `stream.thinking` / `stream.chunk` 事件 → 收尾发 `stream.fragment`（有工具）或 `stream.end`。
- `tools` 节点：`ToolNodeWithEvents` 执行工具，逐条发 `stream.tool_result` 事件并写回
  `tool_results`；内置工具经 `ToolContext::execute` 派发，宿主工具回退到 registry `func`。

流式事件通过 `state.output()`（fastmind `EventBuffer`）**实时**写入，多消费者游标读取。

## LLM 抽象

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat(&self, req: &LlmRequest) -> Result<LlmResponse>;
    async fn chat_stream(&self, req: &LlmRequest, sink: &mut dyn LlmSink) -> Result<LlmResponse>;
}
```

- `LlmRequest` 统一 system / messages / tools / model / max_tokens。
- `LlmSink` 区分 `on_thinking` / `on_text` / `on_tool_call_start`，两条网关的流式回调在此归一。
- 适配器用 **chunk 级流**（`chat_stream_chunks` / `messages_stream_events`）以保持 reasoning 与
  content 分离，工具调用参数做 index 累积。
- 网关由 agent 配置 `llm.gateway` 决定；`build_provider` 无 key 时返回 `None`，agent 发
  `stream.error`；`AgentRuntime` 支持注入 mock provider（测试/无 key 宿主）。

## 数据格式（与 Python 版兼容）

| 文件 | 格式 |
|------|------|
| `data/sessions/sessions.json` | JSON（session_id/agent_id/name/…/work_dir） |
| `data/sessions/<id>/messages.jsonl` | JSONL（role/content/tool_calls/tool_call_id/reasoning_content/timestamp） |
| `data/agents/<id>/metadata.json` + `SOUL.md`/`USER.md`/`AGENT.md` | JSON + Markdown |
| `skills/{bundled,user}/<name>/SKILL.md` | Markdown |

详见根目录设计文档 `fastclaw-rs_design.md`。
