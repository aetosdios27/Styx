use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TaskKind {
    Session,
    Dht,
    Lsd,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SharedWorkerKind {
    Dht,
    Lsd,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureReasonCode {
    Cancelled,
    ChannelClosed,
    CommandBackpressure,
    WorkerPanicked,
    ShutdownTimeout,
    PersistenceFailed,
    DhtFailed,
    LsdFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistenceOutcome {
    NotAttempted,
    Succeeded,
    Failed(FailureReasonCode),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskExit {
    Graceful,
    Failed(FailureReasonCode),
    Aborted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShutdownMode {
    Clean,
    Forced,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShutdownReport {
    pub mode: ShutdownMode,
    pub elapsed: Duration,
    pub exits: BTreeMap<TaskKind, Vec<TaskExit>>,
    pub persistence: PersistenceOutcome,
}

impl ShutdownReport {
    #[must_use]
    pub fn new(mode: ShutdownMode, elapsed: Duration) -> Self {
        Self {
            mode,
            elapsed,
            exits: BTreeMap::new(),
            persistence: PersistenceOutcome::NotAttempted,
        }
    }

    #[must_use]
    pub fn aborted_count(&self) -> usize {
        self.exits
            .values()
            .flatten()
            .filter(|exit| **exit == TaskExit::Aborted)
            .count()
    }
}
