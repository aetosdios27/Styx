use serde::de::DeserializeOwned;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::{
    commands::{CommandResponseEnvelope, ControlCommand},
    error::CliError,
    runtime::TorrentRuntime,
};

pub fn encode_command(command: &ControlCommand) -> Result<Vec<u8>, CliError> {
    encode_line(command)
}

pub fn decode_command(bytes: &[u8]) -> Result<ControlCommand, CliError> {
    decode_exact(bytes)
}

pub fn encode_response(response: &CommandResponseEnvelope) -> Result<Vec<u8>, CliError> {
    encode_line(response)
}

fn encode_line(value: &impl serde::Serialize) -> Result<Vec<u8>, CliError> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    Ok(bytes)
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
    use tokio::net::UnixListener;

    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path)?;
    loop {
        let (stream, _) = listener.accept().await?;
        handle_unix_stream(stream, &mut runtime).await?;
    }
}

#[cfg(not(unix))]
pub async fn serve_unix_socket(
    _path: &std::path::Path,
    _runtime: impl crate::runtime::TorrentRuntime + Send + 'static,
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
    let mut response = Vec::new();
    reader.read_until(b'\n', &mut response).await?;
    decode_exact(&response)
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
    let mut request = Vec::new();
    reader.read_until(b'\n', &mut request).await?;
    let response = match decode_command(&request).and_then(|command| runtime.apply(command)) {
        Ok(response) => CommandResponseEnvelope::ok(response),
        Err(error) => CommandResponseEnvelope::err(error.to_string()),
    };
    let mut stream = reader.into_inner();
    stream.write_all(&encode_response(&response)?).await?;
    stream.shutdown().await?;
    Ok(())
}
