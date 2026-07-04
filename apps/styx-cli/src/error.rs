use thiserror::Error;

#[derive(Debug, Error)]
pub enum CliError {
    #[error(transparent)]
    App(#[from] styx_app::AppError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Runtime(#[from] styx_runtime::RuntimeError),
    #[error("IPC sockets are not supported on this platform")]
    UnsupportedIpc,
    #[error("command is not supported by the memory runtime")]
    UnsupportedMemoryCommand,
}
