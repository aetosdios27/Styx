use std::io::Write;

use crate::{
    error::CliError,
    events::AppEvent,
    runtime::{MemoryRuntime, TorrentRuntime},
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HeadlessOptions {
    pub ipc: Option<String>,
}

pub fn run_headless_once<R, W>(
    runtime: R,
    mut writer: W,
    options: HeadlessOptions,
) -> Result<(), CliError>
where
    R: TorrentRuntime,
    W: Write,
{
    write_event(
        &mut writer,
        &AppEvent::DaemonStarted {
            ipc: options.ipc,
            at_ms: 0,
        },
    )?;
    write_event(
        &mut writer,
        &AppEvent::Snapshot {
            snapshot: runtime.snapshot(),
        },
    )?;
    Ok(())
}

pub fn run_default_headless<W>(writer: W, options: HeadlessOptions) -> Result<(), CliError>
where
    W: Write,
{
    run_headless_once(MemoryRuntime::default(), writer, options)
}

fn write_event(writer: &mut impl Write, event: &AppEvent) -> Result<(), CliError> {
    serde_json::to_writer(&mut *writer, event)?;
    writer.write_all(b"\n")?;
    Ok(())
}
