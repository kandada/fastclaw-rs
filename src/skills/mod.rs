// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Skill subsystem: meta-skills (SKILL.md instructions) + native Rust skills.

pub mod registry;

pub use registry::{Skill, SkillInfo, SkillKind, SkillRegistry};
