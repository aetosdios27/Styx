//! Desktop GUI backend for Styx.

pub mod commands;
pub mod error;
pub mod state;
pub mod tauri_commands;

use anyhow::Result;

pub fn run() -> Result<()> {
    tauri::Builder::<tauri::Wry>::default()
        .manage(state::GuiState::new(6881)?)
        .invoke_handler(tauri::generate_handler![
            tauri_commands::get_snapshot,
            tauri_commands::add_torrent,
            tauri_commands::remove_torrent,
            tauri_commands::pause_torrent,
            tauri_commands::resume_torrent
        ])
        .run(tauri::generate_context!())
        .map_err(Into::into)
}
