//! Command-line and terminal UI boundary for Styx.

pub mod args;
pub mod error;
pub mod headless;
pub mod ipc;
pub mod tui;

use std::{io::Write, str::FromStr};

use anyhow::Result;
use serde_json::json;

use crate::{
    args::{Cli, Command, DaemonCommand},
    error::CliError,
    headless::{run_default_headless, HeadlessOptions},
    ipc::{send_daemon_control, send_unix_command, serve_daemon_socket, DaemonControlCommand},
};
use styx_app::{
    CommandResponseEnvelope, ControlCommand, InfoHashHex, MemoryRuntime, TorrentRuntime,
};
use styx_runtime::{AppRuntime, DaemonConfig, DaemonRuntime, RuntimeConfig, RuntimeEngine};

pub async fn run(cli: Cli) -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .try_init();

    if cli.headless {
        run_default_headless(
            std::io::stdout(),
            HeadlessOptions {
                ipc: cli.ipc.as_ref().map(|path| path.display().to_string()),
                listen_port: cli.listen_port,
            },
        )?;
        return Ok(());
    }

    if let Some(command) = cli.command.as_ref() {
        if let Command::Smoke {
            torrent,
            dest,
            listen_port,
        } = command
        {
            if cli.ipc.is_some() {
                return Err(CliError::UnsupportedMemoryCommand.into());
            }
            let mut config =
                styx_runtime::SmokeRunConfig::default_for_paths(torrent.clone(), dest.clone());
            config.listen_port = *listen_port;
            let outcome = styx_runtime::run_one_piece_smoke(config).await?;
            serde_json::to_writer(
                std::io::stdout(),
                &json!({
                    "ok": true,
                    "response": {
                        "type": "smoke_verified",
                        "piece": outcome.piece(),
                        "bytes": outcome.bytes()
                    }
                }),
            )?;
            println!();
            return Ok(());
        }
        if let Command::Download {
            torrent,
            dest,
            listen_port,
        } = command
        {
            if cli.ipc.is_some() {
                return Err(CliError::UnsupportedMemoryCommand.into());
            }
            let mut config =
                styx_runtime::DownloadRunConfig::default_for_paths(torrent.clone(), dest.clone());
            config.listen_port = *listen_port;
            let outcome = styx_runtime::run_full_v1_download(config).await?;
            serde_json::to_writer(
                std::io::stdout(),
                &json!({
                    "ok": true,
                    "response": {
                        "type": "download_complete",
                        "pieces": outcome.pieces(),
                        "bytes": outcome.bytes()
                    }
                }),
            )?;
            println!();
            return Ok(());
        }
        if let Command::Daemon(command) = command {
            if let DaemonCommand::Start { state_dir, socket } = command {
                let config = RuntimeConfig {
                    listen_port: cli.listen_port,
                    ..RuntimeConfig::default()
                };
                let (daemon, owner) = DaemonRuntime::start(DaemonConfig {
                    state_dir: state_dir.clone(),
                    socket_path: socket.clone(),
                    tick_interval: config.snapshot_interval,
                    runtime_config: config,
                })
                .await?;
                let shutdown = daemon.clone();
                let mut owner_wait = Box::pin(owner.wait());
                let mut socket_server = Box::pin(serve_daemon_socket(socket, daemon));
                tokio::select! {
                    result = &mut socket_server => {
                        let _ = shutdown.request_shutdown().await;
                        result?;
                        let _ = owner_wait.await?;
                    }
                    result = &mut owner_wait => {
                        let _ = result?;
                        socket_server.await?;
                    }
                }
                return Ok(());
            }
            run_daemon_command_once(command.clone(), std::io::stdout()).await?;
            return Ok(());
        }

        if let Some(path) = &cli.ipc {
            let command = control_command(command)?;
            let response = send_unix_command(path, &command).await?;
            serde_json::to_writer(std::io::stdout(), &response)?;
            println!();
        } else {
            run_command_once(cli, std::io::stdout())?;
        }
        return Ok(());
    }

    let config = RuntimeConfig {
        listen_port: cli.listen_port,
        ..RuntimeConfig::default()
    };
    let runtime = AppRuntime::new(RuntimeEngine::new(config)?);
    tui::run_tui(runtime).await?;
    Ok(())
}

pub fn run_command_once(cli: Cli, mut writer: impl Write) -> Result<(), CliError> {
    let Some(command) = cli.command.as_ref() else {
        let response = CommandResponseEnvelope::ok(styx_app::CommandResponse::Status {
            snapshot: MemoryRuntime::default().snapshot(),
        });
        serde_json::to_writer(&mut writer, &response)?;
        writer.write_all(b"\n")?;
        return Ok(());
    };

    let command = control_command(command)?;
    let mut runtime = MemoryRuntime::default();
    let response = match runtime.apply(command) {
        Ok(response) => CommandResponseEnvelope::ok(response),
        Err(error) => CommandResponseEnvelope::err(error.to_string()),
    };
    serde_json::to_writer(&mut writer, &response)?;
    writer.write_all(b"\n")?;
    Ok(())
}

pub async fn run_daemon_command_once(
    command: DaemonCommand,
    mut writer: impl Write,
) -> Result<(), CliError> {
    let response = match command {
        DaemonCommand::Status { socket } => {
            send_daemon_control(&socket, &DaemonControlCommand::Status).await?
        }
        DaemonCommand::Stop { socket } => {
            send_daemon_control(&socket, &DaemonControlCommand::Stop).await?
        }
        DaemonCommand::Start { .. } => {
            return Err(CliError::UnsupportedMemoryCommand);
        }
    };
    serde_json::to_writer(&mut writer, &response)?;
    writer.write_all(b"\n")?;
    Ok(())
}

fn control_command(command: &Command) -> Result<ControlCommand, CliError> {
    Ok(match command {
        Command::Add {
            source,
            destination,
        } => ControlCommand::Add {
            source: source.clone(),
            destination: destination.clone(),
        },
        Command::AddMagnet { uri, destination } => ControlCommand::AddMagnet {
            uri: uri.clone(),
            destination: Some(destination.clone()),
        },
        Command::Remove { info_hash } => ControlCommand::Remove {
            info_hash: InfoHashHex::from_str(info_hash)?,
        },
        Command::Pause { info_hash } => ControlCommand::Pause {
            info_hash: InfoHashHex::from_str(info_hash)?,
        },
        Command::Resume { info_hash } => ControlCommand::Resume {
            info_hash: InfoHashHex::from_str(info_hash)?,
        },
        Command::Status => ControlCommand::Status,
        Command::Smoke { .. } | Command::Download { .. } | Command::Daemon(_) => {
            return Err(CliError::UnsupportedMemoryCommand)
        }
    })
}
