use std::{path::PathBuf, str::FromStr};

use serde::Serialize;
use styx_app::{AppSnapshot, CommandResponse, InfoHashHex};
use tauri::State;

use crate::{commands, error::GuiError, state::GuiState};

#[derive(Debug, Serialize)]
pub struct GuiCommandError {
    message: String,
}

impl From<GuiError> for GuiCommandError {
    fn from(error: GuiError) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

impl From<styx_app::AppError> for GuiCommandError {
    fn from(error: styx_app::AppError) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

#[tauri::command]
pub async fn get_snapshot(state: State<'_, GuiState>) -> Result<AppSnapshot, GuiCommandError> {
    commands::get_snapshot(&state).await.map_err(Into::into)
}

#[tauri::command]
pub async fn add_torrent(
    state: State<'_, GuiState>,
    source: String,
    destination: Option<String>,
) -> Result<CommandResponse, GuiCommandError> {
    commands::add_torrent(
        &state,
        PathBuf::from(source),
        destination.map(PathBuf::from),
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn remove_torrent(
    state: State<'_, GuiState>,
    info_hash: String,
) -> Result<CommandResponse, GuiCommandError> {
    let info_hash = InfoHashHex::from_str(&info_hash)?;
    commands::remove_torrent(&state, info_hash)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn pause_torrent(
    state: State<'_, GuiState>,
    info_hash: String,
) -> Result<CommandResponse, GuiCommandError> {
    let info_hash = InfoHashHex::from_str(&info_hash)?;
    commands::pause_torrent(&state, info_hash)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn resume_torrent(
    state: State<'_, GuiState>,
    info_hash: String,
) -> Result<CommandResponse, GuiCommandError> {
    let info_hash = InfoHashHex::from_str(&info_hash)?;
    commands::resume_torrent(&state, info_hash)
        .await
        .map_err(Into::into)
}
