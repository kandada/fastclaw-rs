// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Error type and error-code mapping.

use fastclaw::{Error, ErrorCode};

#[test]
fn error_code_mapping() {
    assert_eq!(Error::NotFound("x".into()).code(), ErrorCode::NotFound);
    assert_eq!(
        Error::AlreadyExists("x".into()).code(),
        ErrorCode::AlreadyExists
    );
    assert_eq!(
        Error::InvalidConfig("x".into()).code(),
        ErrorCode::InvalidConfig
    );
    assert_eq!(Error::Workspace("x".into()).code(), ErrorCode::Workspace);
    assert_eq!(Error::WorkDir("x".into()).code(), ErrorCode::WorkDir);
    assert_eq!(Error::Permission("x".into()).code(), ErrorCode::Permission);
    assert_eq!(Error::Llm("x".into()).code(), ErrorCode::Llm);
    assert_eq!(Error::Skill("x".into()).code(), ErrorCode::Skill);
    assert_eq!(
        Error::SessionStopped("x".into()).code(),
        ErrorCode::SessionStopped
    );
}

#[test]
fn error_io_and_json_conversion() {
    let io = std::io::Error::new(std::io::ErrorKind::Other, "boom");
    let e = Error::from(io);
    assert_eq!(e.code(), ErrorCode::Io);

    let json = serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
    let e = Error::from(json);
    assert_eq!(e.code(), ErrorCode::Json);
}

#[test]
fn error_fastmind_conversion() {
    // fastmind::Error can be wrapped via `From`.
    let fe = fastclaw::fastmind::Error::NotFound("g".into());
    let e = Error::from(fe);
    assert_eq!(e.code(), ErrorCode::FastMind);
}

#[test]
fn error_display_is_human_readable() {
    let e = Error::Permission("denied".into());
    assert!(e.to_string().contains("denied"));
}
