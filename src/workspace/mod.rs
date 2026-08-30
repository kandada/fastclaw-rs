// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Workspace module: path resolution, working-directory management, session store.

pub mod paths;
pub mod session;
pub mod workdir;

pub use paths::WorkspacePaths;
pub use session::SessionStore;
pub use workdir::WorkDirManager;
