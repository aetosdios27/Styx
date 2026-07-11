use std::collections::BTreeMap;
use std::future;
use std::num::NonZeroUsize;
use std::time::Instant;

use styx_dht::InfoHash;
use styx_proto::InfoHashV1;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::{
    spawn_dht_worker, spawn_lsd_worker, DhtClient, DhtCommand, DhtRuntimeEvent, LsdClient,
    LsdCommand, LsdError, LsdRuntimeEvent, RuntimeConfig, RuntimeError, TorrentId,
};

use super::{
    FailureReasonCode, SharedWorkerKind, ShutdownMode, ShutdownReport, TaskExit, TaskKind,
    TaskRegistry,
};

const SHUTDOWN_CONTROL_GRACE: std::time::Duration = std::time::Duration::from_millis(100);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionNotice {
    DhtBootstrapped {
        nodes: usize,
    },
    SharedWorkerFailed {
        worker: SharedWorkerKind,
        reason: FailureReasonCode,
    },
}

#[derive(Clone, Debug)]
pub struct SessionClient {
    commands: mpsc::Sender<SessionCommand>,
}

pub struct SessionEventStream {
    events: mpsc::Receiver<SessionEvent>,
    notices: mpsc::Receiver<SessionNotice>,
}

pub struct SessionOwner {
    shutdown: Option<oneshot::Sender<ShutdownRequest>>,
    join: Option<JoinHandle<()>>,
    clean_timeout: std::time::Duration,
    forced_timeout: std::time::Duration,
}

#[derive(Debug)]
enum SessionCommand {
    Dht(DhtCommand),
    Lsd(LsdCommand),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SessionEvent {
    Dht(DhtRuntimeEvent),
    Lsd(LsdRuntimeEvent),
    SharedWorkerFailed {
        worker: SharedWorkerKind,
        reason: FailureReasonCode,
    },
}

struct ShutdownRequest {
    mode: ShutdownMode,
    reply: oneshot::Sender<ShutdownReport>,
}

struct SessionSupervisor {
    dht: Option<DhtClient>,
    lsd: Option<LsdClient>,
    dht_events: Option<mpsc::Receiver<DhtRuntimeEvent>>,
    lsd_events: Option<mpsc::Receiver<LsdRuntimeEvent>>,
    registry: TaskRegistry,
    commands: Option<mpsc::Receiver<SessionCommand>>,
    events: mpsc::Sender<SessionEvent>,
    notices: mpsc::Sender<SessionNotice>,
    shutdown: oneshot::Receiver<ShutdownRequest>,
    clean_timeout: std::time::Duration,
    forced_timeout: std::time::Duration,
    observed_exits: BTreeMap<TaskKind, Vec<TaskExit>>,
    capability_failures: BTreeMap<SharedWorkerKind, Vec<FailureReasonCode>>,
    session_failures: Vec<FailureReasonCode>,
}

impl SessionClient {
    pub fn bootstrap_dht(&self) -> Result<(), RuntimeError> {
        self.try_send(SessionCommand::Dht(DhtCommand::Bootstrap))
    }

    pub fn get_peers(&self, torrent: TorrentId, info_hash: InfoHash) -> Result<(), RuntimeError> {
        self.try_send(SessionCommand::Dht(DhtCommand::GetPeers {
            torrent,
            info_hash,
        }))
    }

    pub fn announce_peer(
        &self,
        torrent: TorrentId,
        info_hash: InfoHash,
        port: u16,
        implied_port: bool,
    ) -> Result<(), RuntimeError> {
        self.try_send(SessionCommand::Dht(DhtCommand::AnnouncePeer {
            torrent,
            info_hash,
            port,
            implied_port,
        }))
    }

    pub fn update_lsd(&self, torrents: Vec<(TorrentId, InfoHashV1)>) -> Result<(), RuntimeError> {
        self.try_send(SessionCommand::Lsd(LsdCommand::Update { torrents }))
    }

    fn try_send(&self, command: SessionCommand) -> Result<(), RuntimeError> {
        self.commands
            .try_send(command)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => RuntimeError::Backpressure {
                    stage: "session_command",
                },
                mpsc::error::TrySendError::Closed(_) => RuntimeError::Cancelled,
            })
    }
}

impl SessionEventStream {
    pub async fn recv(&mut self) -> Option<SessionNotice> {
        loop {
            tokio::select! {
                notice = self.notices.recv() => return notice,
                event = self.events.recv() => {
                    if event.is_none() {
                        return self.notices.recv().await;
                    }
                }
            }
        }
    }

    #[expect(
        dead_code,
        reason = "consumed by the Task 7 AppRuntime adapter migration"
    )]
    pub(crate) fn try_recv_event(&mut self) -> Result<SessionEvent, mpsc::error::TryRecvError> {
        self.events.try_recv()
    }
}

impl Drop for SessionEventStream {
    fn drop(&mut self) {
        self.events.close();
        self.notices.close();
    }
}

impl SessionOwner {
    pub async fn shutdown(self, mode: ShutdownMode) -> Result<ShutdownReport, RuntimeError> {
        let mut owner = self;
        let worker_budget = match mode {
            ShutdownMode::Clean => owner
                .clean_timeout
                .checked_add(owner.forced_timeout)
                .unwrap_or(std::time::Duration::MAX),
            ShutdownMode::Forced => owner.forced_timeout,
        };
        let total_budget = worker_budget
            .checked_add(SHUTDOWN_CONTROL_GRACE)
            .unwrap_or(std::time::Duration::MAX);
        let deadline = deadline_after(total_budget);
        let started = Instant::now();
        let (reply, receiver) = oneshot::channel();
        let sender = owner.shutdown.take().ok_or(RuntimeError::Cancelled)?;
        sender
            .send(ShutdownRequest { mode, reply })
            .map_err(|_| RuntimeError::Cancelled)?;
        let report = match tokio::time::timeout_at(deadline, receiver).await {
            Ok(Ok(report)) => report,
            Ok(Err(_)) => return Err(RuntimeError::Cancelled),
            Err(_) => return Ok(owner.abort_with_timeout_report(started.elapsed())),
        };
        if let Some(mut join) = owner.join.take() {
            match tokio::time::timeout_at(deadline, &mut join).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => return Err(RuntimeError::Cancelled),
                Err(_) => {
                    join.abort();
                    return Ok(timeout_report(started.elapsed()));
                }
            }
        }
        Ok(report)
    }

    fn abort_with_timeout_report(&mut self, elapsed: std::time::Duration) -> ShutdownReport {
        if let Some(join) = self.join.take() {
            join.abort();
        }
        timeout_report(elapsed)
    }
}

impl Drop for SessionOwner {
    fn drop(&mut self) {
        if let Some(join) = &self.join {
            join.abort();
        }
    }
}

pub async fn spawn_session_supervisor(
    config: RuntimeConfig,
) -> Result<(SessionClient, SessionEventStream, SessionOwner), RuntimeError> {
    let config = config.validate()?;
    let session = config.session;
    let (command_tx, command_rx) = mpsc::channel(session.command_capacity);
    let (event_tx, event_rx) = mpsc::channel(session.event_capacity);
    let (notice_tx, notice_rx) = mpsc::channel(session.event_capacity);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let mut registry = TaskRegistry::default();

    let (dht, dht_events) = if config.dht.enabled {
        let (events_tx, events_rx) = mpsc::channel(session.event_capacity);
        let (client, owner) = spawn_dht_worker(config.dht.clone(), events_tx).await?;
        registry.register(owner.into_task());
        (Some(client), Some(events_rx))
    } else {
        (None, None)
    };

    let (lsd_events_tx, lsd_events_rx) = mpsc::channel(session.event_capacity);
    let lsd = spawn_lsd_worker(
        config.listen_port,
        NonZeroUsize::new(session.command_capacity).ok_or(RuntimeError::InvalidConfig(
            "session command capacity must be greater than zero",
        ))?,
        lsd_events_tx,
    );
    let (lsd, lsd_events) = match lsd {
        Some((client, owner)) => {
            registry.register(owner.into_task());
            (Some(client), Some(lsd_events_rx))
        }
        None => (None, None),
    };

    let supervisor = SessionSupervisor {
        dht,
        lsd,
        dht_events,
        lsd_events,
        registry,
        commands: Some(command_rx),
        events: event_tx,
        notices: notice_tx,
        shutdown: shutdown_rx,
        clean_timeout: session.shutdown_timeout,
        forced_timeout: session.forced_shutdown_timeout,
        observed_exits: BTreeMap::new(),
        capability_failures: BTreeMap::new(),
        session_failures: Vec::new(),
    };
    let join = tokio::spawn(supervisor.run());

    Ok((
        SessionClient {
            commands: command_tx,
        },
        SessionEventStream {
            events: event_rx,
            notices: notice_rx,
        },
        SessionOwner {
            shutdown: Some(shutdown_tx),
            join: Some(join),
            clean_timeout: session.shutdown_timeout,
            forced_timeout: session.forced_shutdown_timeout,
        },
    ))
}

impl SessionSupervisor {
    async fn run(mut self) {
        let mut completion_poll = tokio::time::interval(std::time::Duration::from_millis(10));
        loop {
            tokio::select! {
                biased;
                request = &mut self.shutdown => {
                    if let Ok(request) = request {
                        self.finish_shutdown(request).await;
                    }
                    break;
                }
                command = recv_optional(&mut self.commands) => {
                    match command {
                        Some(command) => self.handle_command(command),
                        None => self.commands = None,
                    }
                }
                event = recv_optional(&mut self.dht_events) => {
                    match event {
                        Some(event) => self.handle_dht_event(event),
                        None => self.dht_events = None,
                    }
                }
                event = recv_optional(&mut self.lsd_events) => {
                    match event {
                        Some(event) => self.handle_lsd_event(event),
                        None => self.lsd_events = None,
                    }
                }
                _ = completion_poll.tick() => self.harvest_finished().await,
            }
        }
    }

    fn handle_command(&mut self, command: SessionCommand) {
        let worker = command_worker(&command);
        let result = match command {
            SessionCommand::Dht(command) => self
                .dht
                .as_ref()
                .ok_or(RuntimeError::Cancelled)
                .and_then(|client| client.try_send(command)),
            SessionCommand::Lsd(command) => self
                .lsd
                .as_ref()
                .ok_or(RuntimeError::Cancelled)
                .and_then(|client| client.try_send(command).map_err(map_lsd_error)),
        };
        if let Err(error) = result {
            let reason = match error {
                RuntimeError::Backpressure { .. } => FailureReasonCode::CommandBackpressure,
                _ => FailureReasonCode::ChannelClosed,
            };
            self.emit_failure(worker, reason);
        }
    }

    fn handle_dht_event(&mut self, event: DhtRuntimeEvent) {
        match normalize_dht_event(event) {
            Err(reason) => {
                self.emit_failure(SharedWorkerKind::Dht, reason);
            }
            Ok((event, notice)) => {
                self.try_emit_event(event);
                if let Some(notice) = notice {
                    self.try_emit_notice(notice);
                }
            }
        }
    }

    fn handle_lsd_event(&mut self, event: LsdRuntimeEvent) {
        self.try_emit_event(SessionEvent::Lsd(event));
    }

    fn emit_failure(&mut self, worker: SharedWorkerKind, reason: FailureReasonCode) {
        push_unique(self.capability_failures.entry(worker).or_default(), reason);
        self.try_emit_event(SessionEvent::SharedWorkerFailed { worker, reason });
        self.try_emit_notice(SessionNotice::SharedWorkerFailed { worker, reason });
    }

    fn try_emit_event(&mut self, event: SessionEvent) {
        if let Err(error) = self.events.try_send(event) {
            self.record_event_delivery_failure(error);
        }
    }

    fn try_emit_notice(&mut self, notice: SessionNotice) {
        if let Err(error) = self.notices.try_send(notice) {
            self.record_event_delivery_failure(error);
        }
    }

    fn record_event_delivery_failure<T>(&mut self, error: mpsc::error::TrySendError<T>) {
        let reason = match error {
            mpsc::error::TrySendError::Full(_) => FailureReasonCode::CommandBackpressure,
            mpsc::error::TrySendError::Closed(_) => FailureReasonCode::ChannelClosed,
        };
        push_unique(&mut self.session_failures, reason);
    }

    async fn harvest_finished(&mut self) {
        let exits = self.registry.drain_finished().await;
        for (kind, task_exits) in exits {
            for exit in &task_exits {
                if let TaskExit::Failed(reason) = exit {
                    if let Some(worker) = shared_worker(kind) {
                        self.try_emit_notice(SessionNotice::SharedWorkerFailed {
                            worker,
                            reason: *reason,
                        });
                    }
                }
            }
            self.observed_exits
                .entry(kind)
                .or_default()
                .extend(task_exits);
        }
    }

    async fn finish_shutdown(&mut self, request: ShutdownRequest) {
        let started = Instant::now();
        let mut exits = self
            .registry
            .shutdown(request.mode, self.clean_timeout, self.forced_timeout)
            .await;
        merge_exits(&mut exits, std::mem::take(&mut self.observed_exits));
        let mut report = ShutdownReport::new(request.mode, started.elapsed());
        report.exits = exits;
        report.capability_failures = std::mem::take(&mut self.capability_failures);
        report.session_failures = std::mem::take(&mut self.session_failures);
        let _ = request.reply.send(report);
    }
}

async fn recv_optional<T>(receiver: &mut Option<mpsc::Receiver<T>>) -> Option<T> {
    match receiver {
        Some(receiver) => receiver.recv().await,
        None => future::pending().await,
    }
}

fn map_lsd_error(error: LsdError) -> RuntimeError {
    match error {
        LsdError::CommandBackpressure => RuntimeError::Backpressure {
            stage: "lsd_command",
        },
        LsdError::WorkerClosed => RuntimeError::Cancelled,
        _ => RuntimeError::Cancelled,
    }
}

fn normalize_dht_event(
    event: DhtRuntimeEvent,
) -> Result<(SessionEvent, Option<SessionNotice>), FailureReasonCode> {
    match event {
        DhtRuntimeEvent::Failed { .. } => Err(FailureReasonCode::DhtFailed),
        DhtRuntimeEvent::Bootstrapped { nodes } => Ok((
            SessionEvent::Dht(DhtRuntimeEvent::Bootstrapped { nodes }),
            Some(SessionNotice::DhtBootstrapped { nodes }),
        )),
        event => Ok((SessionEvent::Dht(event), None)),
    }
}

fn command_worker(command: &SessionCommand) -> SharedWorkerKind {
    match command {
        SessionCommand::Dht(_) => SharedWorkerKind::Dht,
        SessionCommand::Lsd(_) => SharedWorkerKind::Lsd,
    }
}

const fn shared_worker(kind: TaskKind) -> Option<SharedWorkerKind> {
    match kind {
        TaskKind::Dht => Some(SharedWorkerKind::Dht),
        TaskKind::Lsd => Some(SharedWorkerKind::Lsd),
        TaskKind::Session => None,
    }
}

fn merge_exits(
    target: &mut BTreeMap<TaskKind, Vec<TaskExit>>,
    source: BTreeMap<TaskKind, Vec<TaskExit>>,
) {
    for (kind, exits) in source {
        target.entry(kind).or_default().extend(exits);
    }
}

fn push_unique<T: Eq>(values: &mut Vec<T>, value: T) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn deadline_after(duration: std::time::Duration) -> tokio::time::Instant {
    let now = tokio::time::Instant::now();
    let mut bounded = duration;
    loop {
        if let Some(deadline) = now.checked_add(bounded) {
            return deadline;
        }
        bounded /= 2;
    }
}

fn timeout_report(elapsed: std::time::Duration) -> ShutdownReport {
    let mut report = ShutdownReport::new(ShutdownMode::Forced, elapsed);
    report.exits.insert(
        TaskKind::Session,
        vec![TaskExit::Failed(FailureReasonCode::ShutdownTimeout)],
    );
    report
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::OwnedTask;

    #[test]
    fn session_client_reports_backpressure_when_command_channel_is_full() {
        let (commands, _receiver) = mpsc::channel(1);
        let client = SessionClient { commands };
        client.bootstrap_dht().unwrap();

        let error = client.bootstrap_dht().unwrap_err();

        assert!(matches!(
            error,
            RuntimeError::Backpressure {
                stage: "session_command"
            }
        ));
    }

    #[test]
    fn dht_failure_text_is_discarded_at_supervision_boundary() {
        let normalized = normalize_dht_event(DhtRuntimeEvent::Failed {
            reason: "peer 203.0.113.7 leaked magnet:?xt=urn:btih:secret".into(),
        });

        assert_eq!(normalized, Err(FailureReasonCode::DhtFailed));
        let debug = format!("{normalized:?}");
        assert!(!debug.contains("203.0.113.7"));
        assert!(!debug.contains("magnet:"));
    }

    #[tokio::test]
    async fn panicked_shared_worker_emits_redacted_failure_notice() {
        let mut registry = TaskRegistry::default();
        registry.register(OwnedTask::new(
            TaskKind::Dht,
            tokio::spawn(async { panic!("synthetic panic with peer 203.0.113.7") }),
        ));
        let (mut supervisor, mut notices, _events, _shutdown) = test_supervisor(registry);
        tokio::task::yield_now().await;

        supervisor.harvest_finished().await;
        let notice = notices.try_recv().unwrap();

        assert_eq!(
            notice,
            SessionNotice::SharedWorkerFailed {
                worker: SharedWorkerKind::Dht,
                reason: FailureReasonCode::WorkerPanicked,
            }
        );
        assert!(!format!("{notice:?}").contains("203.0.113.7"));
    }

    #[tokio::test]
    async fn forced_shutdown_reports_stalled_registered_worker() {
        let mut registry = TaskRegistry::default();
        registry.register(OwnedTask::new(
            TaskKind::Lsd,
            tokio::spawn(future::pending::<()>()),
        ));
        let (mut supervisor, _notices, _events, _shutdown) = test_supervisor(registry);
        let (reply, report) = oneshot::channel();

        supervisor
            .finish_shutdown(ShutdownRequest {
                mode: ShutdownMode::Forced,
                reply,
            })
            .await;
        let report = report.await.unwrap();

        assert_eq!(report.exits[&TaskKind::Lsd], vec![TaskExit::Aborted]);
    }

    #[tokio::test]
    async fn operational_failure_is_not_misreported_as_worker_exit() {
        let (mut supervisor, _notices, _events, _shutdown) =
            test_supervisor(TaskRegistry::default());
        supervisor.emit_failure(SharedWorkerKind::Dht, FailureReasonCode::DhtFailed);
        let (reply, report) = oneshot::channel();

        supervisor
            .finish_shutdown(ShutdownRequest {
                mode: ShutdownMode::Clean,
                reply,
            })
            .await;
        let report = report.await.unwrap();

        assert!(!report.exits.contains_key(&TaskKind::Dht));
        assert_eq!(
            report.capability_failures[&SharedWorkerKind::Dht],
            vec![FailureReasonCode::DhtFailed]
        );
    }

    fn test_supervisor(
        registry: TaskRegistry,
    ) -> (
        SessionSupervisor,
        mpsc::Receiver<SessionNotice>,
        mpsc::Receiver<SessionEvent>,
        oneshot::Sender<ShutdownRequest>,
    ) {
        let (_commands, command_rx) = mpsc::channel(1);
        let (events, event_rx) = mpsc::channel(4);
        let (notices, notice_rx) = mpsc::channel(4);
        let (shutdown_tx, shutdown) = oneshot::channel();
        (
            SessionSupervisor {
                dht: None,
                lsd: None,
                dht_events: None,
                lsd_events: None,
                registry,
                commands: Some(command_rx),
                events,
                notices,
                shutdown,
                clean_timeout: Duration::from_millis(10),
                forced_timeout: Duration::from_millis(10),
                observed_exits: BTreeMap::new(),
                capability_failures: BTreeMap::new(),
                session_failures: Vec::new(),
            },
            notice_rx,
            event_rx,
            shutdown_tx,
        )
    }
}
