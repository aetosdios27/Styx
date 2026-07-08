use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use styx_app::{commands::CommandResponse, error::AppError, ControlCommand, TorrentRuntime};
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::{PersistentAppRuntime, PersistentStore, RuntimeConfig, RuntimeError};

#[derive(Clone, Debug)]
pub struct DaemonConfig {
    pub state_dir: PathBuf,
    pub socket_path: PathBuf,
    pub tick_interval: Duration,
    pub runtime_config: RuntimeConfig,
}

#[derive(Clone, Debug)]
pub struct DaemonStatus {
    pub pid: Option<u32>,
    pub socket_path: PathBuf,
    pub torrent_count: u32,
    pub uptime: Duration,
}

#[derive(Debug)]
pub struct DaemonRuntime;

#[derive(Clone, Debug)]
pub struct DaemonHandle {
    tx: mpsc::Sender<DaemonRequest>,
    join: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

#[derive(Debug)]
enum DaemonRequest {
    Apply {
        command: ControlCommand,
        reply: oneshot::Sender<Result<CommandResponse, AppError>>,
    },
    Status {
        reply: oneshot::Sender<Result<DaemonStatus, RuntimeError>>,
    },
    Shutdown {
        reply: oneshot::Sender<Result<(), RuntimeError>>,
    },
}

impl DaemonRuntime {
    pub async fn start(config: DaemonConfig) -> Result<DaemonHandle, RuntimeError> {
        if config.tick_interval.is_zero() {
            return Err(RuntimeError::InvalidConfig(
                "daemon tick_interval must be greater than zero",
            ));
        }
        let store = PersistentStore::open(&config.state_dir)?;
        let runtime = PersistentAppRuntime::open(config.runtime_config.clone(), store).await?;
        let (tx, rx) = mpsc::channel(64);
        let join = tokio::spawn(run_daemon(config, runtime, rx));
        Ok(DaemonHandle {
            tx,
            join: Arc::new(Mutex::new(Some(join))),
        })
    }
}

impl DaemonHandle {
    pub async fn apply(&self, command: ControlCommand) -> Result<CommandResponse, AppError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(DaemonRequest::Apply { command, reply })
            .await
            .map_err(|_| AppError::Internal("daemon command channel closed".into()))?;
        rx.await
            .map_err(|_| AppError::Internal("daemon command response dropped".into()))?
    }

    pub async fn status(&self) -> Result<DaemonStatus, RuntimeError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(DaemonRequest::Status { reply })
            .await
            .map_err(|_| RuntimeError::Cancelled)?;
        rx.await.map_err(|_| RuntimeError::Cancelled)?
    }

    pub async fn shutdown(self) -> Result<(), RuntimeError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(DaemonRequest::Shutdown { reply })
            .await
            .map_err(|_| RuntimeError::Cancelled)?;
        let result = rx.await.map_err(|_| RuntimeError::Cancelled)?;
        if let Some(join) = self.join.lock().await.take() {
            let _ = join.await;
        }
        result
    }
}

async fn run_daemon(
    config: DaemonConfig,
    mut runtime: PersistentAppRuntime,
    mut rx: mpsc::Receiver<DaemonRequest>,
) {
    let started = Instant::now();
    let mut interval = tokio::time::interval(config.tick_interval);
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let _ = runtime.tick_and_persist();
            }
            Some(request) = rx.recv() => {
                match request {
                    DaemonRequest::Apply { command, reply } => {
                        let _ = reply.send(runtime.apply_and_persist(command));
                    }
                    DaemonRequest::Status { reply } => {
                        let _ = reply.send(Ok(status_from_runtime(&mut runtime, &config, started)));
                    }
                    DaemonRequest::Shutdown { reply } => {
                        let result = runtime.persist_now();
                        let _ = reply.send(result);
                        break;
                    }
                }
            }
            else => break,
        }
    }
}

fn status_from_runtime(
    runtime: &mut PersistentAppRuntime,
    config: &DaemonConfig,
    started: Instant,
) -> DaemonStatus {
    let snapshot = runtime.runtime_mut().snapshot();
    DaemonStatus {
        pid: Some(std::process::id()),
        socket_path: config.socket_path.clone(),
        torrent_count: snapshot.torrents.len() as u32,
        uptime: started.elapsed(),
    }
}
