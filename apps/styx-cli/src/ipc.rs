use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use styx_app::{CommandResponseEnvelope, ControlCommand, TorrentRuntime};
use styx_runtime::{DaemonHandle, DaemonStatus};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::error::CliError;

const MAX_IPC_FRAME_BYTES: usize = 64 * 1024;

pub fn encode_command(command: &ControlCommand) -> Result<Vec<u8>, CliError> {
    encode_line(command)
}

pub fn decode_command(bytes: &[u8]) -> Result<ControlCommand, CliError> {
    decode_exact(bytes)
}

pub fn encode_response(response: &CommandResponseEnvelope) -> Result<Vec<u8>, CliError> {
    encode_line(response)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonControlCommand {
    #[serde(rename = "daemon_status")]
    Status,
    #[serde(rename = "daemon_stop")]
    Stop,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonControlResponse {
    #[serde(rename = "daemon_status")]
    Status {
        pid: Option<u32>,
        socket_path: String,
        torrent_count: u32,
        uptime_ms: u64,
    },
    #[serde(rename = "daemon_stopped")]
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DaemonResponseEnvelope {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<DaemonControlResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl DaemonResponseEnvelope {
    #[must_use]
    pub fn ok(response: DaemonControlResponse) -> Self {
        Self {
            ok: true,
            response: Some(response),
            error: None,
        }
    }

    #[must_use]
    pub fn err(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            response: None,
            error: Some(error.into()),
        }
    }
}

fn encode_line(value: &impl serde::Serialize) -> Result<Vec<u8>, CliError> {
    let mut writer = LimitedFrameWriter::new(MAX_IPC_FRAME_BYTES - 1);
    let result = serde_json::to_writer(&mut writer, value);
    if writer.exceeded {
        return Err(CliError::IpcFrameTooLarge {
            max: MAX_IPC_FRAME_BYTES,
        });
    }
    result?;
    let mut bytes = writer.bytes;
    bytes.push(b'\n');
    Ok(bytes)
}

struct LimitedFrameWriter {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}

impl LimitedFrameWriter {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(limit.min(4096)),
            limit,
            exceeded: false,
        }
    }
}

impl std::io::Write for LimitedFrameWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let remaining = self.limit.saturating_sub(self.bytes.len());
        if buffer.len() > remaining {
            self.exceeded = true;
            return Err(std::io::Error::new(
                std::io::ErrorKind::FileTooLarge,
                "IPC frame limit exceeded",
            ));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn decode_exact<T>(bytes: &[u8]) -> Result<T, CliError>
where
    T: DeserializeOwned,
{
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = T::deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(value)
}

#[cfg(unix)]
pub async fn serve_unix_socket(
    path: &std::path::Path,
    mut runtime: impl TorrentRuntime + Send + 'static,
) -> Result<(), CliError> {
    let listener = bind_secure_unix_listener(path)?;
    loop {
        let (stream, _) = listener.accept().await?;
        handle_unix_stream(stream, &mut runtime).await?;
    }
}

#[cfg(not(unix))]
pub async fn serve_unix_socket(
    _path: &std::path::Path,
    _runtime: impl TorrentRuntime + Send + 'static,
) -> Result<(), CliError> {
    Err(CliError::UnsupportedIpc)
}

#[cfg(unix)]
pub async fn serve_daemon_socket(
    path: &std::path::Path,
    daemon: DaemonHandle,
) -> Result<(), CliError> {
    let listener = bind_secure_unix_listener(path)?;
    loop {
        let (stream, _) = listener.accept().await?;
        let daemon = daemon.clone();
        tokio::spawn(async move {
            let _ = handle_daemon_stream(stream, daemon).await;
        });
    }
}

#[cfg(not(unix))]
pub async fn serve_daemon_socket(
    _path: &std::path::Path,
    _daemon: DaemonHandle,
) -> Result<(), CliError> {
    Err(CliError::UnsupportedIpc)
}

#[cfg(unix)]
pub async fn send_unix_command(
    path: &std::path::Path,
    command: &ControlCommand,
) -> Result<CommandResponseEnvelope, CliError> {
    use tokio::net::UnixStream;

    let mut stream = UnixStream::connect(path).await?;
    stream.write_all(&encode_command(command)?).await?;
    stream.shutdown().await?;
    let mut reader = BufReader::new(stream);
    let response = read_ipc_frame(&mut reader).await?;
    decode_exact(&response)
}

#[cfg(unix)]
pub async fn send_daemon_control(
    path: &std::path::Path,
    command: &DaemonControlCommand,
) -> Result<DaemonResponseEnvelope, CliError> {
    use tokio::net::UnixStream;

    let mut stream = UnixStream::connect(path).await?;
    stream.write_all(&encode_line(command)?).await?;
    stream.shutdown().await?;
    let mut reader = BufReader::new(stream);
    let response = read_ipc_frame(&mut reader).await?;
    decode_exact(&response)
}

#[cfg(not(unix))]
pub async fn send_daemon_control(
    _path: &std::path::Path,
    _command: &DaemonControlCommand,
) -> Result<DaemonResponseEnvelope, CliError> {
    Err(CliError::UnsupportedIpc)
}

#[cfg(unix)]
async fn handle_daemon_stream(
    stream: tokio::net::UnixStream,
    daemon: DaemonHandle,
) -> Result<(), CliError> {
    let mut reader = BufReader::new(stream);
    let request = read_ipc_frame(&mut reader).await?;
    if let Ok(command) = decode_exact::<DaemonControlCommand>(&request) {
        let response = handle_daemon_control(command, daemon).await;
        let mut stream = reader.into_inner();
        stream.write_all(&encode_line(&response)?).await?;
        stream.shutdown().await?;
        return Ok(());
    }
    let response = match decode_command(&request) {
        Ok(command) => match daemon.apply(command).await {
            Ok(response) => CommandResponseEnvelope::ok(response),
            Err(error) => CommandResponseEnvelope::err(error.to_string()),
        },
        Err(error) => CommandResponseEnvelope::err(error.to_string()),
    };
    let mut stream = reader.into_inner();
    stream.write_all(&encode_response(&response)?).await?;
    stream.shutdown().await?;
    Ok(())
}

async fn handle_daemon_control(
    command: DaemonControlCommand,
    daemon: DaemonHandle,
) -> DaemonResponseEnvelope {
    match command {
        DaemonControlCommand::Status => match daemon.status().await {
            Ok(status) => DaemonResponseEnvelope::ok(status_response(status)),
            Err(error) => DaemonResponseEnvelope::err(error.to_string()),
        },
        DaemonControlCommand::Stop => match daemon.shutdown().await {
            Ok(()) => DaemonResponseEnvelope::ok(DaemonControlResponse::Stopped),
            Err(error) => DaemonResponseEnvelope::err(error.to_string()),
        },
    }
}

fn status_response(status: DaemonStatus) -> DaemonControlResponse {
    DaemonControlResponse::Status {
        pid: status.pid,
        socket_path: status.socket_path.display().to_string(),
        torrent_count: status.torrent_count,
        uptime_ms: status.uptime.as_millis().min(u128::from(u64::MAX)) as u64,
    }
}

#[cfg(not(unix))]
pub async fn send_unix_command(
    _path: &std::path::Path,
    _command: &ControlCommand,
) -> Result<CommandResponseEnvelope, CliError> {
    Err(CliError::UnsupportedIpc)
}

#[cfg(unix)]
async fn handle_unix_stream<R>(
    stream: tokio::net::UnixStream,
    runtime: &mut R,
) -> Result<(), CliError>
where
    R: TorrentRuntime,
{
    let mut reader = BufReader::new(stream);
    let request = read_ipc_frame(&mut reader).await?;
    let response = match decode_command(&request)
        .and_then(|command| runtime.apply(command).map_err(crate::error::CliError::from))
    {
        Ok(response) => CommandResponseEnvelope::ok(response),
        Err(error) => CommandResponseEnvelope::err(error.to_string()),
    };
    let mut stream = reader.into_inner();
    stream.write_all(&encode_response(&response)?).await?;
    stream.shutdown().await?;
    Ok(())
}

async fn read_ipc_frame<R>(reader: &mut R) -> Result<Vec<u8>, CliError>
where
    R: AsyncRead + Unpin,
{
    let mut frame = Vec::with_capacity(MAX_IPC_FRAME_BYTES.min(4096));
    reader
        .take((MAX_IPC_FRAME_BYTES + 1) as u64)
        .read_to_end(&mut frame)
        .await?;
    if frame.len() > MAX_IPC_FRAME_BYTES {
        return Err(CliError::IpcFrameTooLarge {
            max: MAX_IPC_FRAME_BYTES,
        });
    }
    if !frame.ends_with(b"\n") {
        return Err(CliError::UnterminatedIpcFrame);
    }
    Ok(frame)
}

#[cfg(unix)]
fn bind_secure_unix_listener(path: &std::path::Path) -> Result<tokio::net::UnixListener, CliError> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};

    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "IPC path has no parent")
    })?;
    std::fs::create_dir_all(parent)?;
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    let parent_uid = std::fs::metadata(parent)?.uid();
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_socket() || metadata.uid() != parent_uid {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "refusing to replace unsafe IPC path",
                )
                .into());
            }
            std::fs::remove_file(path)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let listener = tokio::net::UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}
