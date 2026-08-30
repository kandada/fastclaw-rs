// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! Unified error type for fastclaw.

/// Error codes used across the FFI boundary (stable integers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ErrorCode {
    Unknown = 1,
    NotFound = 2,
    AlreadyExists = 3,
    InvalidConfig = 4,
    Workspace = 5,
    WorkDir = 6,
    Permission = 7,
    Llm = 8,
    Skill = 9,
    Io = 10,
    SessionStopped = 11,
    FastMind = 12,
    Json = 13,
}

/// The crate-wide error type.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    AlreadyExists(String),
    #[error("{0}")]
    InvalidConfig(String),
    #[error("{0}")]
    Workspace(String),
    #[error("{0}")]
    WorkDir(String),
    #[error("{0}")]
    Permission(String),
    #[error("{0}")]
    Llm(String),
    #[error("{0}")]
    Skill(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    SessionStopped(String),
    #[error(transparent)]
    FastMind(#[from] fastmind::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl Error {
    /// Stable integer code for FFI / error-string mapping.
    pub fn code(&self) -> ErrorCode {
        match self {
            Error::NotFound(_) => ErrorCode::NotFound,
            Error::AlreadyExists(_) => ErrorCode::AlreadyExists,
            Error::InvalidConfig(_) => ErrorCode::InvalidConfig,
            Error::Workspace(_) => ErrorCode::Workspace,
            Error::WorkDir(_) => ErrorCode::WorkDir,
            Error::Permission(_) => ErrorCode::Permission,
            Error::Llm(_) => ErrorCode::Llm,
            Error::Skill(_) => ErrorCode::Skill,
            Error::Io(_) => ErrorCode::Io,
            Error::SessionStopped(_) => ErrorCode::SessionStopped,
            Error::FastMind(_) => ErrorCode::FastMind,
            Error::Json(_) => ErrorCode::Json,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
