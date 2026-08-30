# 工作目录（WorkDir）

工作目录子系统把 fastclaw-rs 从「无状态 CLI」升级为对齐 aacode 等编程工具的「项目目录」体验：
**不同任务可指定不同目录，未指定时回落默认目录**。

## 概念

| 概念 | 含义 |
|------|------|
| Workspace 根 | fastclaw 自身元数据区（会话/skills/agents/settings） |
| 工作目录 | 任务真正读写、产出文件的地方（用户项目目录） |
| 默认工作目录 | 未指定时的落点 |

## 三级解析

```
resolve(session_id, agent.work_dirs):
  1. 会话显式绑定 work_dir        # bind_work_dir(session_id, dir)
  2. agent 配置 work_dirs 首位    # metadata.json 的 work_dirs（兼容旧名 extra_workspaces）
  3. 全局默认 default_work_dir    # settings.json；缺省 = 启动时 current_dir()
```

## API

```rust
claw.register_work_dir(&path)?;        // 加入信任集合
claw.unregister_work_dir(&path)?;
claw.list_work_dirs();                 // Vec<PathBuf>
claw.set_default_work_dir(&path)?;
claw.default_work_dir();
claw.bind_work_dir(&session, &path)?;  // 会话绑定
claw.unbind_work_dir(&session);
claw.resolve_work_dir(&session, None); // 三级解析
```

## 持久化

- `register_work_dir` / `unregister_work_dir`：同步写 `settings.json` 的 `work_dirs`。
- `set_default_work_dir`：同步写 `settings.json` 的 `default_work_dir`。
- `bind_work_dir`：把绑定写入 `sessions.json` 该会话条目的 `work_dir` 字段；
  `unbind_work_dir` 移除之。启动时 `FastClaw::build` 自动恢复所有已持久化的会话绑定。
- TUI：`/workdir <path>` 与启动参数 `--workdir <path>` 均通过 `set_default_work_dir`
  持久化，重启后保留项目目录。

## 与 run_shell / 安全的联动

- `run_shell` 以 `resolve()` 得到的目录为子进程 `cwd`，相对路径天然落在目录内。
- 安全判定中「默认目录 + 全部注册目录 + agent work_dirs」内的路径 → `allow`；越界到系统路径
  （`/etc/` `/usr/` 等）→ `ask_user`（`CONFIRM:` 确认）。
- SystemPrompt 注入当前工作目录与可用目录列表，引导 LLM 用相对路径。

## Workspace 根解析（桌面 vs 移动）

```
resolve_workspace_root(config):
  config.workspace_root 显式注入  → 移动端/嵌入式（沙盒目录）
  FASTCLAW_WORKSPACE 环境变量     → 显式迁移/共享
  cfg!(debug_assertions) 下的 FASTCLAW_DEV_WORKSPACE → 开发默认（<repo>/workspace，build.rs 烘焙）
  否则                            → ~/.fastclaw-rs/workspace
```

桌面端默认根与 Python 版（`~/.fastclaw/workspace`）错开，避免冲突；格式层 100% 兼容，`FASTCLAW_WORKSPACE`
指向旧目录即可「原地接管」迁移。共享同一目录时用 `data/.lock` 文件锁互斥（`fs2`）。
