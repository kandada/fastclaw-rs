// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Agent module: ReAct loop, system prompt, routing.

#[allow(clippy::module_inception)]
pub mod agent;
pub mod system_prompt;

pub use agent::{build_graph, route, AgentRuntime};
