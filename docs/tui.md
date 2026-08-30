# TUI 使用说明

`fastclaw-tui` 是一个交互式终端界面，用于开发/测试 fastclaw-rs 的 agent。

## 运行

```bash
# 在 fastclaw-rs 目录下
cargo run --bin fastclaw-tui --features tui
```

（`tui` 是 Cargo feature，需显式开启；发布构建：`cargo build --release --features tui` 后运行 `target/release/fastclaw-tui`。）

## 前置：配置 LLM

TUI 需要一个能用的 agent（含 API key）。两种方式任选：

**方式一：环境变量**（seed 的 `main_agent` 会回退读取）

```bash
export LLM_API_KEY="sk-..."
export LLM_API_URL="https://api.deepseek.com/v1"
export LLM_MODEL_NAME="deepseek-v4-flash"
```

**方式二：workspace 的 agent 配置**（推荐，可持久化）

编辑 `<workspace>/data/agents/<agent_id>/metadata.json`，填入 `llm.api_key` 等。

Workspace 位置：
- 开发构建：仓库根下的 `workspace/`（`<repo>/workspace`，已 git 忽略）
- 发布/安装：`~/.fastclaw-rs/workspace`
- 可用 `--workspace <path>` 显式指定

## 启动参数

| 参数 | 说明 |
|------|------|
| `--workspace <path>` | 指定 workspace 根目录 |
| `--agent <id>` | 指定默认 agent（默认 `main_agent`） |
| `--session <id>` | 续接指定会话（默认新建） |

示例：

```bash
cargo run --bin fastclaw-tui --features tui -- \
  --workspace ~/.fastclaw-rs/workspace \
  --agent minimax-anthropic
```

## 斜杠命令

| 命令 | 说明 |
|------|------|
| `/new` | 新建会话 |
| `/clear` | 清空当前会话历史（含磁盘） |
| `/session <id>` | 切换到指定会话 |
| `/session_list` | 列出所有会话 |
| `/agent <id>` | 切换 agent |
| `/skill list` | 列出可用技能 |
| `/workdir <path>` | 切换并注册工作目录 |
| `/help` | 显示命令列表 |
| `/quit` | 退出 |

## 键盘操作

| 键 | 说明 |
|----|------|
| `Enter` | 发送消息（生成中按回车不发送，内容保留在输入框） |
| `←` / `→` / `Home` / `End` | 输入框光标移动 |
| `Backspace` / `Delete` | 删除字符 |
| `Ctrl+L` | 强制整屏重绘（画面错乱时用） |
| `Ctrl+C` / `Esc` | 退出 |

- 粘贴 / 输入法提交的文本会直接进入输入框。
- 窗口大小随时可以调整，布局会自动重排。

## 界面说明

- 顶部对话区：自动滚动到底部；不同角色着色区分
  - 黄色加粗 = 用户
  - 默认色 = 助手回复（流式逐字输出）
  - 青色斜体 = 思考（reasoning）
  - 品红 = 工具调用 / 工具结果
  - 灰色 = 系统信息；红色 = 错误
- 底部 `You` 输入框：**钉死在屏幕最底部**，自动换行，行高随内容自适应（最高 5 行），超长时框内滚动、光标始终可见；调整窗口大小布局自动重排
- 输入框标题右侧显示状态：`ready` / `generating…`（生成中带旋转动画）；生成中最后一条助手回复末尾有一个闪烁光标块
- 标题栏显示当前 `session / agent / 工作目录名`（用 basename，保持紧凑）

## 端到端测试

`scripts/tui_e2e.exp` 用 expect/PTY 驱动真实 TUI，覆盖完整交互（真实 LLM）：

1. 启动渲染 greeting
2. 发送消息 → 验证**流式回复**（用模型会输出的 "Tokyo"/"Paris"，不在输入里，避免输入回显误判）
3. 触发 `run_shell` 工具 → 验证 `[Executing tool]` 与 `[run_shell] <结果>` 渲染
4. 斜杠命令 `/help` `/skill list` `/new` `/workdir` `/clear` `/session_list`
5. 干净退出

运行：

```bash
cargo build --bin fastclaw-tui --features tui
expect scripts/tui_e2e.exp    # 需要 workspace 里有真实 LLM key
```
