# 宿主接入指南（桌面 / 移动）

fastclaw-rs 是可嵌入库，公共 API 刻意保持 **FFI 友好**（无泛型、无生命周期、JSON 用 `String` 承载）。

## 核心约束

1. **公共边界无泛型/生命周期**：跨 FFI 的类型为 `String` / `Vec<String>` / `i64` / `bool` / opaque 句柄。
2. **流式走回调**：`run_chat(session, text, on_chunk, on_end)` 或 `stream_events`（`StreamEvent` 扁平结构）。
3. **统一错误**：`Error` 带稳定错误码 `ErrorCode`，FFI 层映射为 `(code: i32, message: String)`。
4. **workspace 根注入**：移动端必须传 `FastClawConfig.workspace_root = 沙盒目录`。

## 桌面端（Tauri）

- 直接以 Rust 依赖集成，Tauri command 里 `Arc<FastClaw>` 直调；或 sidecar 进程 + IPC。
- TUI 用 `fastclaw-tui`（`features = ["tui"]`）。

```rust
let claw = Arc::new(FastClaw::builder(FastClawConfig {
    workspace_root: Some(app_data_dir),
    ..FastClawConfig::default()
}).build());
claw.start().await?;
```

## 移动端（iOS / Android / 鸿蒙）

- 注入沙盒目录作为 workspace 根（iOS `Application Support`、Android `context.filesDir`、鸿蒙沙盒）。
- **iOS 沙盒不能 fork/exec**，`run_shell` 基本不可用 → 通过 `FastClawConfig.extra_tools` 注入「设备能力工具」
  （沙盒文件读写、HTTP、系统 API 等）；ReAct 图与 SystemPrompt 对工具无感知。
- 桥接方案（文档级建议，库不内置）：**UniFFI** / **flutter_rust_bridge** / **napi-rs**，三选一。

```rust
// 设备能力工具示例
let read_file = fastclaw::fastmind::Tool::new(
    "read_file", "read a file inside the sandbox",
    Arc::new(|args, _ctx| Box::pin(async move {
        let p = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
        Ok(serde_json::json!(std::fs::read_to_string(p).unwrap_or_default()))
    })),
);
let claw = FastClaw::builder(FastClawConfig {
    workspace_root: Some(sandbox_dir),
    extra_tools: vec![read_file],
    ..FastClawConfig::default()
}).build();
```

## 回调式流（FFI 风格）

```rust
claw.run_chat(
    &session,
    "hello",
    Some(Arc::new(|delta: String| { /* on_chunk */ })),
    Some(Arc::new(|full: String| { /* on_end */ })),
).await;
```

## System prompt 注入（宿主能力感知）

人格（`SOUL.md` / `USER.md` / `AGENT.md`）是**按会话绑定、绑定后缓存**的静态内容。若宿主需要在
运行时动态注入 system prompt（如告诉 agent 本宿主有哪些能力、命令怎么用），请用注入接口——它存在
`FastClaw` 实例内存中，**每轮 LLM 请求现读现拼**，下一次请求即生效，无需重绑会话，天然按
Profile（一个 Profile = 一个 `FastClaw` 实例）隔离。

```rust
use fastclaw::PromptSection;

// 覆盖整组注入章节
claw.set_injections(vec![PromptSection::new(
    "浏览器能力（fastbrowser）",
    "你有浏览器内核 fastbrowser，可用 run_shell 命令操作网页：\n\
     - 感知：fastbrowser snapshot\n\
     - 动作：fastbrowser click a / type e \"文字\"\n\
     - 截图：fastbrowser screenshot <路径>\n\
     完整命令表：fastbrowser --help",
)]);

// 或追加单章
claw.append_injection(PromptSection::new("安全提示", "不要运行 rm -rf /"));

// 读取当前注入
let sections: Vec<PromptSection> = claw.injections();
```

注入章节渲染在 system prompt 的**personality 之后**，以 `## {title}` + content 追加；空 content 跳过。
内存版已满足多数宿主（重启后由宿主重新注入）；如需要持久化到 `metadata.json`，可在
`AgentConfig` 上自行扩展。

完整可运行示例见 `examples/mobile_api.rs`。
