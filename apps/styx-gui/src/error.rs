use thiserror::Error;

#[derive(Debug, Error)]
pub enum GuiError {
    #[error(transparent)]
    App(#[from] styx_app::AppError),
}
