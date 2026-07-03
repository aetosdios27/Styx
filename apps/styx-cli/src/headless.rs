use std::io::Write;

use styx_app::{AppError, AppEvent, MemoryRuntime, TorrentRuntime};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HeadlessOptions {
    pub ipc: Option<String>,
}

pub fn run_headless_once<R, W>(
    runtime: R,
    mut writer: W,
    options: HeadlessOptions,
) -> Result<(), AppError>
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

pub fn run_default_headless<W>(writer: W, options: HeadlessOptions) -> Result<(), AppError>
where
    W: Write,
{
    run_headless_once(MemoryRuntime::default(), writer, options)
}

fn write_event(writer: &mut impl Write, event: &AppEvent) -> Result<(), AppError> {
    serde_json::to_writer(&mut *writer, event)?;
    writer.write_all(b"\n")?;
    Ok(())
}
