use thiserror::Error;

#[derive(Debug, Error)]
pub enum NativeCommandError {
    #[error("native device is disconnected")]
    Offline,
    #[error("native command queue is full")]
    QueueFull,
}

#[derive(Debug, Error)]
pub enum NativeError {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("TLS: {0}")]
    Tls(#[from] openssl::error::ErrorStack),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid peer frame")]
    InvalidFrame,
    #[error("received-share quota exceeded")]
    QuotaExceeded,
    #[error("unknown pending peer or comparison code")]
    UnknownPending,
    #[error("discovery: {0}")]
    Discovery(String),
}
