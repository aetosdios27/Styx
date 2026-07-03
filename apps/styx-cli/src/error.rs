use thiserror::Error;

#[derive(Debug, Error)]
pub enum CliError {
    #[error(transparent)]
    App(#[from] styx_app::AppError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("IPC sockets are not supported on this platform")]
    UnsupportedIpc,
}
