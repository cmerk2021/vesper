use thiserror::Error;

/// Errors produced by vesper-core.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("the stored or provided token was rejected (401)")]
    Unauthorized,

    #[error("Discord returned an unexpected status: {status}")]
    UnexpectedStatus { status: u16 },

    #[error("websocket error: {0}")]
    WebSocket(#[from] Box<tokio_tungstenite::tungstenite::Error>),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("keyring error: {0}")]
    Keyring(String),

    #[error("theme error: {0}")]
    Theme(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
