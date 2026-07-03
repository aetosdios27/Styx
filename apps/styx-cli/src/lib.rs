//! Command-line and terminal UI boundary for Styx.

pub mod args;
pub mod commands;
pub mod error;
pub mod events;
pub mod format;
pub mod headless;
pub mod ipc;
pub mod model;
pub mod runtime;
pub mod tui;

use std::{io::Write, str::FromStr};

use anyhow::Result;

use crate::{
    args::{Cli, Command},
    commands::{CommandResponseEnvelope, ControlCommand},
    error::CliError,
    headless::{run_default_headless, HeadlessOptions},
    ipc::send_unix_command,
    runtime::{MemoryRuntime, TorrentRuntime},
};

pub async fn run(cli: Cli) -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .try_init();

    if cli.headless {
        run_default_headless(
            std::io::stdout(),
            HeadlessOptions {
                ipc: cli.ipc.as_ref().map(|path| path.display().to_string()),
            },
        )?;
        return Ok(());
    }

    if let Some(command) = cli.command.as_ref() {
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

    tui::run_tui(MemoryRuntime::default()).await?;
    Ok(())
}

pub fn run_command_once(cli: Cli, mut writer: impl Write) -> Result<(), CliError> {
    let Some(command) = cli.command.as_ref() else {
        let response = CommandResponseEnvelope::ok(crate::commands::CommandResponse::Status {
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

fn control_command(command: &Command) -> Result<ControlCommand, CliError> {
    Ok(match command {
        Command::Add {
            source,
            destination,
        } => ControlCommand::Add {
            source: source.clone(),
            destination: destination.clone(),
        },
        Command::Remove { info_hash } => ControlCommand::Remove {
            info_hash: crate::format::InfoHashHex::from_str(info_hash)?,
        },
        Command::Pause { info_hash } => ControlCommand::Pause {
            info_hash: crate::format::InfoHashHex::from_str(info_hash)?,
        },
        Command::Resume { info_hash } => ControlCommand::Resume {
            info_hash: crate::format::InfoHashHex::from_str(info_hash)?,
        },
        Command::Status => ControlCommand::Status,
    })
}
