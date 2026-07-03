use std::path::PathBuf;

use styx_app::{AppSnapshot, CommandResponse, ControlCommand, InfoHashHex, TorrentRuntime};

use crate::{error::GuiError, state::GuiState};

pub async fn get_snapshot(state: &GuiState) -> Result<AppSnapshot, GuiError> {
    Ok(state.with_runtime(|runtime| runtime.snapshot()).await)
}

pub async fn add_torrent(
    state: &GuiState,
    source: PathBuf,
    destination: Option<PathBuf>,
) -> Result<CommandResponse, GuiError> {
    apply(
        state,
        ControlCommand::Add {
            source,
            destination,
        },
    )
    .await
}

pub async fn remove_torrent(
    state: &GuiState,
    info_hash: InfoHashHex,
) -> Result<CommandResponse, GuiError> {
    apply(state, ControlCommand::Remove { info_hash }).await
}

pub async fn pause_torrent(
    state: &GuiState,
    info_hash: InfoHashHex,
) -> Result<CommandResponse, GuiError> {
    apply(state, ControlCommand::Pause { info_hash }).await
}

pub async fn resume_torrent(
    state: &GuiState,
    info_hash: InfoHashHex,
) -> Result<CommandResponse, GuiError> {
    apply(state, ControlCommand::Resume { info_hash }).await
}

async fn apply(state: &GuiState, command: ControlCommand) -> Result<CommandResponse, GuiError> {
    state
        .with_runtime(|runtime| runtime.apply(command))
        .await
        .map_err(GuiError::from)
}
