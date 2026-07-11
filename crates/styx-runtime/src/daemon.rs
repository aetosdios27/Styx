use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use styx_app::{commands::CommandResponse, error::AppError, ControlCommand, TorrentRuntime};
use tokio::sync::{mpsc, oneshot, watch};

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
pub struct DaemonClient {
    tx: mpsc::Sender<DaemonRequest>,
    completion: watch::Receiver<bool>,
}

#[derive(Debug)]
pub struct DaemonOwner {
    join: Option<tokio::task::JoinHandle<Result<crate::ShutdownReport, RuntimeError>>>,
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
    pub async fn start(config: DaemonConfig) -> Result<(DaemonClient, DaemonOwner), RuntimeError> {
        if config.tick_interval.is_zero() {
            return Err(RuntimeError::InvalidConfig(
                "daemon tick_interval must be greater than zero",
            ));
        }
        let store = PersistentStore::open(&config.state_dir)?;
        let mut runtime = PersistentAppRuntime::open(config.runtime_config.clone(), store).await?;
        let (session, session_events, session_owner) =
            crate::spawn_session_supervisor(config.runtime_config.clone()).await?;
        runtime
            .runtime_mut()
            .attach_session(session, session_events)?;
        let (tx, rx) = mpsc::channel(64);
        let (completion_tx, completion_rx) = watch::channel(false);
        let join = tokio::spawn(async move {
            let result = run_daemon(config, runtime, rx, session_owner).await;
            let _ = completion_tx.send(true);
            result
        });
        Ok((
            DaemonClient {
                tx,
                completion: completion_rx,
            },
            DaemonOwner { join: Some(join) },
        ))
    }
}

impl DaemonClient {
    pub async fn stopping(&self) {
        self.tx.closed().await;
    }

    pub async fn completed(&self) {
        let mut completion = self.completion.clone();
        while !*completion.borrow() {
            if completion.changed().await.is_err() {
                break;
            }
        }
    }

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

    pub async fn request_shutdown(&self) -> Result<(), RuntimeError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(DaemonRequest::Shutdown { reply })
            .await
            .map_err(|_| RuntimeError::Cancelled)?;
        rx.await.map_err(|_| RuntimeError::Cancelled)?
    }
}

impl DaemonOwner {
    pub async fn wait(mut self) -> Result<crate::ShutdownReport, RuntimeError> {
        let join = self.join.take().ok_or(RuntimeError::Cancelled)?;
        join.await.map_err(|_| RuntimeError::Cancelled)?
    }

    pub async fn abort(mut self) -> crate::ShutdownReport {
        let started = Instant::now();
        if let Some(join) = self.join.take() {
            if join.is_finished() {
                if let Ok(Ok(report)) = join.await {
                    return report;
                }
                return aborted_daemon_report(started.elapsed());
            }
            join.abort();
            let _ = join.await;
        }
        aborted_daemon_report(started.elapsed())
    }
}

impl Drop for DaemonOwner {
    fn drop(&mut self) {
        if let Some(join) = &self.join {
            join.abort();
        }
    }
}

async fn run_daemon(
    config: DaemonConfig,
    mut runtime: PersistentAppRuntime,
    mut rx: mpsc::Receiver<DaemonRequest>,
    session_owner: crate::SessionOwner,
) -> Result<crate::ShutdownReport, RuntimeError> {
    let started = Instant::now();
    let mut interval = tokio::time::interval(config.tick_interval);
    let shutdown_reply = loop {
        tokio::select! {
            _ = interval.tick() => {
                let _ = runtime.tick_and_persist();
            }
            request = rx.recv() => {
                match request {
                    None => break None,
                    Some(DaemonRequest::Apply { command, reply }) => {
                        let _ = reply.send(runtime.apply_and_persist(command));
                    }
                    Some(DaemonRequest::Status { reply }) => {
                        let _ = reply.send(Ok(status_from_runtime(&mut runtime, &config, started)));
                    }
                    Some(DaemonRequest::Shutdown { reply }) => {
                        rx.close();
                        break Some(reply);
                    }
                }
            }
            else => break None,
        }
    };
    runtime.quiesce().await;
    let session = session_owner.shutdown(crate::ShutdownMode::Clean).await;
    let persistence = runtime.persist_now();
    let mut report = match session {
        Ok(report) => report,
        Err(error) => {
            let mut report =
                crate::ShutdownReport::new(crate::ShutdownMode::Clean, started.elapsed());
            report.persistence = match persistence {
                Ok(()) => crate::PersistenceOutcome::Succeeded,
                Err(_) => {
                    crate::PersistenceOutcome::Failed(crate::FailureReasonCode::PersistenceFailed)
                }
            };
            let failure = RuntimeError::DaemonShutdown {
                reason: crate::FailureReasonCode::ChannelClosed,
                report: Box::new(report),
            };
            if let Some(reply) = shutdown_reply {
                let _ = reply.send(Err(RuntimeError::Cancelled));
            }
            let _ = error;
            return Err(failure);
        }
    };
    report.persistence = match &persistence {
        Ok(()) => crate::PersistenceOutcome::Succeeded,
        Err(_) => crate::PersistenceOutcome::Failed(crate::FailureReasonCode::PersistenceFailed),
    };
    if let Some(reply) = shutdown_reply {
        let acknowledgement = persistence
            .as_ref()
            .map(|_| ())
            .map_err(|_| RuntimeError::Persistence("final daemon persistence failed"));
        let _ = reply.send(acknowledgement);
    }
    match persistence {
        Ok(()) => Ok(report),
        Err(_) => Err(RuntimeError::DaemonShutdown {
            reason: crate::FailureReasonCode::PersistenceFailed,
            report: Box::new(report),
        }),
    }
}

fn aborted_daemon_report(elapsed: Duration) -> crate::ShutdownReport {
    let mut report = crate::ShutdownReport::new(crate::ShutdownMode::Forced, elapsed);
    report
        .exits
        .insert(crate::TaskKind::Session, vec![crate::TaskExit::Aborted]);
    report.persistence = crate::PersistenceOutcome::NotAttempted;
    report
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
