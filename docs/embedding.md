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

完整可运行示例见 `examples/mobile_api.rs`。
