use std::io::Write;

use styx_app::{AppError, AppEvent, TorrentRuntime};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadlessOptions {
    pub ipc: Option<String>,
    pub listen_port: u16,
}

impl Default for HeadlessOptions {
    fn default() -> Self {
        Self {
            ipc: None,
            listen_port: 6881,
        }
    }
}

pub fn run_headless_once<R, W>(
    mut runtime: R,
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
    let config = styx_runtime::RuntimeConfig {
        listen_port: options.listen_port,
        ..styx_runtime::RuntimeConfig::default()
    };
    let engine =
        styx_runtime::RuntimeEngine::new(config).map_err(|e| AppError::Internal(e.to_string()))?;
    let runtime = styx_runtime::AppRuntime::new(engine);
    run_headless_once(runtime, writer, options)
}

fn write_event(writer: &mut impl Write, event: &AppEvent) -> Result<(), AppError> {
    serde_json::to_writer(&mut *writer, event)?;
    writer.write_all(b"\n")?;
    Ok(())
}
