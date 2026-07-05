//! Desktop GUI backend for Styx.

pub mod commands;
pub mod error;
pub mod state;
pub mod tauri_commands;

use std::time::Duration;

use anyhow::Result;
use tauri::{Emitter, Manager};

pub fn run() -> Result<()> {
    let state = state::GuiState::new(6881)?;

    tauri::Builder::<tauri::Wry>::default()
        .manage(state)
        .setup(|app| {
            let handle = app.handle().clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_millis(250));
                loop {
                    interval.tick().await;
                    let Some(state) = handle.try_state::<state::GuiState>() else {
                        continue;
                    };
                    let events = state.tick().await;
                    for event in &events {
                        let _ = handle.emit("styx://event", event);
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            tauri_commands::get_snapshot,
            tauri_commands::add_torrent,
            tauri_commands::remove_torrent,
            tauri_commands::pause_torrent,
            tauri_commands::resume_torrent,
        ])
        .run(tauri::generate_context!())
        .map_err(Into::into)
}
