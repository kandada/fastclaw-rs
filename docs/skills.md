# Skills

fastclaw-rs 的技能子系统支持两类技能。

## 两类技能

| 类型 | 说明 | 执行方式 |
|------|------|----------|
| **元技能（Meta）** | 一个含 `SKILL.md` 的目录（+ 可选资源文件） | 返回 `SKILL.md` 指引 + 资源清单，agent 读后自行用 `run_shell` 完成步骤 |
| **原生技能（Native）** | 实现 `Skill` trait 的 Rust 代码 | 直接执行 |

## 目录布局

```
<workspace>/skills/
├── bundled/            # 内置技能
│   └── <skill>/SKILL.md
└── user/               # 用户自定义技能（`_` 前缀目录忽略）
    ├── <skill>/SKILL.md
    └── _template/      # 用户技能模板（seed 提供，registry 不加载）
```

## 内置文件 seed

首次启动（或升级后新增）时，`seed::seed_workspace`（见 `src/seed.rs`）会把
`workspace_seed/` 下的真实文件写入用户 workspace，文件级保护：目标文件已存在则
跳过、绝不覆盖（对标 Python `bootstrap.copy_seed_files`）。

`workspace_seed/` 布局：

```
workspace_seed/
├── skills/
│   ├── bundled/skill_creator/SKILL.md   # 预置技能
│   └── user/_template/SKILL.md          # 用户技能模板
└── data/agents/main_agent/              # 默认 agent
    ├── SOUL.md / USER.md / AGENT.md / metadata.json
```

种子文件以 `include_str!` 在编译期内嵌进二进制（`seed::SEED_FILES` 清单），
桌面 / 移动 / 嵌入式宿主无需随包携带文件树；新增种子 = 在 `workspace_seed/`
加文件 + 在清单加一行。

预置技能：

- **skill_creator** — 「创建技能的技能」：指导 agent 如何创建 / 更新 / 优化技能
  （命名规则、SKILL.md 结构、heredoc 写入、`__list__` / `__info__` 校验等）。

## SKILL.md 格式

```markdown
---
description: "一句话简介（可选，YAML frontmatter）"
---

## Description
技能的详细说明与执行步骤

## Parameters
（可选）参数说明

## Example
（可选）示例
```

## 原生技能

```rust
use async_trait::async_trait;
use fastclaw::Skill;
use serde_json::Value;

struct MySkill;

#[async_trait]
impl Skill for MySkill {
    fn name(&self) -> &str { "my_skill" }
    fn description(&self) -> &str { "…" }
    async fn execute(&self, params: Value) -> fastclaw::Result<String> {
        Ok("done".to_string())
    }
}
```

内置示例：`current_time`（见 `src/api.rs`）。

## run_skills 工具

- `run_skills("__list__")`：列出全部技能。
- `run_skills("__info__", {"skill_name": "x"})`：查看 `SKILL.md`（不执行）。
- `run_skills("x")`：执行技能（元技能返回指引，原生技能执行）。
- `run_skills("x", timeout=120)`：超时控制。
