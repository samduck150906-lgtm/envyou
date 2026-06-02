//! Error types for envyou-core.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("crypto error: {0}")]
    Crypto(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("license error: {0}")]
    License(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("limit reached: {0}")]
    LimitReached(String),

    #[error("approval denied by user")]
    ApprovalDenied,

    #[error("config error: {0}")]
    Config(String),
}
