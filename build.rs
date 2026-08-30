// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

use std::path::Path;

fn main() {
    // Development workspace: only in debug builds, bake the repo-root
    // `workspace/` directory into the binary so `cargo run` needs no env vars.
    //
    // CARGO_MANIFEST_DIR = <repo>/fastclaw-rs; its parent is the repo root
    // (which also contains `fastmind-rs/`). We rely on `PROFILE` rather than
    // `debug_assertions` because build scripts are compiled with their own
    // profile, not the target crate's.
    let is_debug = std::env::var("PROFILE")
        .map(|p| p == "debug")
        .unwrap_or(false);
    if is_debug {
        if let Ok(dir) = std::env::var("CARGO_MANIFEST_DIR") {
            let root = Path::new(&dir).parent().unwrap_or_else(|| Path::new("."));
            println!(
                "cargo:rustc-env=FASTCLAW_DEV_WORKSPACE={}/workspace",
                root.display()
            );
        }
    }
}
