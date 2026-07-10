use std::collections::BTreeMap;
use std::time::Duration;

use tokio::task::{JoinError, JoinHandle};
use tokio::time::{timeout_at, Instant};

use super::{FailureReasonCode, ShutdownMode, TaskExit, TaskKind};

#[must_use = "owned tasks must be registered or explicitly shut down"]
pub struct OwnedTask {
    kind: TaskKind,
    handle: JoinHandle<()>,
}

impl OwnedTask {
    #[cfg(test)]
    fn new(kind: TaskKind, handle: JoinHandle<()>) -> Self {
        Self { kind, handle }
    }
}

impl Drop for OwnedTask {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

#[derive(Default)]
pub struct TaskRegistry {
    tasks: BTreeMap<TaskKind, Vec<OwnedTask>>,
}

impl TaskRegistry {
    pub fn register(&mut self, task: OwnedTask) {
        self.tasks.entry(task.kind).or_default().push(task);
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    pub async fn shutdown(
        &mut self,
        mode: ShutdownMode,
        clean_timeout: Duration,
        forced_timeout: Duration,
    ) -> BTreeMap<TaskKind, Vec<TaskExit>> {
        let mut pending = self.take_all();
        let mut exits = BTreeMap::new();

        if mode == ShutdownMode::Clean {
            let deadline = deadline_after(clean_timeout);
            let mut first_pending = pending.len();
            for (index, task) in pending.iter_mut().enumerate() {
                match timeout_at(deadline, &mut task.handle).await {
                    Ok(result) => push_exit(&mut exits, task.kind, classify_join(result, false)),
                    Err(_) => {
                        first_pending = index;
                        break;
                    }
                }
            }
            pending.drain(..first_pending);
        }

        let mut unfinished = Vec::with_capacity(pending.len());
        for mut task in pending {
            if task.handle.is_finished() {
                let result = (&mut task.handle).await;
                push_exit(&mut exits, task.kind, classify_join(result, false));
            } else {
                unfinished.push(task);
            }
        }

        for task in &unfinished {
            task.handle.abort();
        }

        let deadline = deadline_after(forced_timeout);
        for mut task in unfinished {
            let exit = match timeout_at(deadline, &mut task.handle).await {
                Ok(result) => classify_join(result, true),
                Err(_) => TaskExit::Failed(FailureReasonCode::ShutdownTimeout),
            };
            push_exit(&mut exits, task.kind, exit);
        }

        exits
    }

    fn take_all(&mut self) -> Vec<OwnedTask> {
        std::mem::take(&mut self.tasks)
            .into_values()
            .flatten()
            .collect()
    }
}

impl Drop for TaskRegistry {
    fn drop(&mut self) {
        for task in self.tasks.values().flatten() {
            task.handle.abort();
        }
    }
}

fn deadline_after(duration: Duration) -> Instant {
    let now = Instant::now();
    let mut bounded = duration;
    loop {
        if let Some(deadline) = now.checked_add(bounded) {
            return deadline;
        }
        bounded /= 2;
    }
}

fn classify_join(result: Result<(), JoinError>, aborted: bool) -> TaskExit {
    match result {
        Ok(()) => TaskExit::Graceful,
        Err(error) if error.is_panic() => TaskExit::Failed(FailureReasonCode::WorkerPanicked),
        Err(error) if error.is_cancelled() && aborted => TaskExit::Aborted,
        Err(_) => TaskExit::Failed(FailureReasonCode::Cancelled),
    }
}

fn push_exit(exits: &mut BTreeMap<TaskKind, Vec<TaskExit>>, kind: TaskKind, exit: TaskExit) {
    exits.entry(kind).or_default().push(exit);
}

#[cfg(test)]
mod tests {
    use std::future;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    use tokio::sync::oneshot;

    use super::*;

    fn register(registry: &mut TaskRegistry, kind: TaskKind, handle: JoinHandle<()>) {
        registry.register(OwnedTask::new(kind, handle));
    }

    #[tokio::test]
    async fn clean_shutdown_joins_every_registered_task() {
        let mut registry = TaskRegistry::default();
        register(&mut registry, TaskKind::Dht, tokio::spawn(async {}));
        register(&mut registry, TaskKind::Lsd, tokio::spawn(async {}));

        let exits = registry
            .shutdown(
                ShutdownMode::Clean,
                Duration::from_secs(1),
                Duration::from_secs(1),
            )
            .await;

        assert_eq!(exits[&TaskKind::Dht], vec![TaskExit::Graceful]);
        assert_eq!(exits[&TaskKind::Lsd], vec![TaskExit::Graceful]);
        assert!(registry.is_empty());
    }

    #[tokio::test]
    async fn panicked_task_is_reported_without_panicking_supervisor() {
        let mut registry = TaskRegistry::default();
        register(
            &mut registry,
            TaskKind::Dht,
            tokio::spawn(async { panic!("synthetic panic") }),
        );

        let exits = registry
            .shutdown(
                ShutdownMode::Clean,
                Duration::from_secs(1),
                Duration::from_secs(1),
            )
            .await;

        assert_eq!(
            exits[&TaskKind::Dht],
            vec![TaskExit::Failed(FailureReasonCode::WorkerPanicked)]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn clean_deadline_aborts_all_remaining_tasks_with_deterministic_evidence() {
        let started = Instant::now();
        let mut registry = TaskRegistry::default();
        register(&mut registry, TaskKind::Dht, tokio::spawn(async {}));
        register(
            &mut registry,
            TaskKind::Lsd,
            tokio::spawn(future::pending::<()>()),
        );
        register(
            &mut registry,
            TaskKind::Lsd,
            tokio::spawn(future::pending::<()>()),
        );

        let shutdown = tokio::spawn(async move {
            registry
                .shutdown(
                    ShutdownMode::Clean,
                    Duration::from_secs(1),
                    Duration::from_secs(1),
                )
                .await
        });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(1)).await;
        let exits = shutdown.await.expect("registry shutdown task must join");

        assert_eq!(Instant::now() - started, Duration::from_secs(1));
        assert_eq!(exits[&TaskKind::Dht], vec![TaskExit::Graceful]);
        assert_eq!(
            exits[&TaskKind::Lsd],
            vec![TaskExit::Aborted, TaskExit::Aborted]
        );
    }

    #[tokio::test]
    async fn cancellation_completed_before_shutdown_is_not_attributed_to_registry_abort() {
        for mode in [ShutdownMode::Clean, ShutdownMode::Forced] {
            let handle = tokio::spawn(future::pending::<()>());
            let abort = handle.abort_handle();
            let mut registry = TaskRegistry::default();
            register(&mut registry, TaskKind::Dht, handle);
            abort.abort();
            tokio::task::yield_now().await;

            let exits = registry
                .shutdown(mode, Duration::from_secs(1), Duration::from_secs(1))
                .await;

            assert_eq!(
                exits[&TaskKind::Dht],
                vec![TaskExit::Failed(FailureReasonCode::Cancelled)]
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn forced_shutdown_skips_clean_deadline_for_live_task() {
        let (started_tx, started_rx) = oneshot::channel();
        let mut registry = TaskRegistry::default();
        register(
            &mut registry,
            TaskKind::Dht,
            tokio::spawn(async move {
                started_tx.send(()).expect("test receiver must remain open");
                future::pending::<()>().await;
            }),
        );
        started_rx.await.expect("registered task must start");
        let started = Instant::now();

        let exits = registry
            .shutdown(
                ShutdownMode::Forced,
                Duration::from_secs(3_600),
                Duration::from_secs(1),
            )
            .await;

        assert_eq!(Instant::now() - started, Duration::ZERO);
        assert_eq!(exits[&TaskKind::Dht], vec![TaskExit::Aborted]);
    }

    #[tokio::test]
    async fn huge_deadlines_do_not_panic() {
        let mut registry = TaskRegistry::default();
        register(&mut registry, TaskKind::Session, tokio::spawn(async {}));

        let exits = registry
            .shutdown(ShutdownMode::Clean, Duration::MAX, Duration::MAX)
            .await;

        assert_eq!(exits[&TaskKind::Session], vec![TaskExit::Graceful]);
    }

    #[test]
    fn owned_task_is_not_cloneable() {
        static_assertions::assert_not_impl_any!(OwnedTask: Clone);
    }

    #[tokio::test]
    async fn dropping_owned_task_aborts_instead_of_detaching() {
        let (started_tx, started_rx) = oneshot::channel();
        let (dropped_tx, dropped_rx) = oneshot::channel();
        let task = OwnedTask::new(
            TaskKind::Session,
            tokio::spawn(async move {
                struct DropSignal(Option<oneshot::Sender<()>>);
                impl Drop for DropSignal {
                    fn drop(&mut self) {
                        if let Some(sender) = self.0.take() {
                            let _ = sender.send(());
                        }
                    }
                }

                let _signal = DropSignal(Some(dropped_tx));
                started_tx.send(()).expect("test receiver must remain open");
                future::pending::<()>().await;
            }),
        );
        started_rx.await.expect("owned task must start");

        drop(task);

        tokio::time::timeout(Duration::from_secs(1), dropped_rx)
            .await
            .expect("owned task state must be dropped after abort")
            .expect("drop signal sender must run");
    }

    #[tokio::test]
    async fn dropping_registry_aborts_instead_of_detaching_registered_tasks() {
        struct DropSignal(Arc<AtomicBool>);

        impl Drop for DropSignal {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let dropped = Arc::new(AtomicBool::new(false));
        let signal = DropSignal(Arc::clone(&dropped));
        let (started_tx, started_rx) = oneshot::channel();
        let mut registry = TaskRegistry::default();
        register(
            &mut registry,
            TaskKind::Session,
            tokio::spawn(async move {
                let _signal = signal;
                started_tx.send(()).expect("test receiver must remain open");
                future::pending::<()>().await;
            }),
        );
        started_rx.await.expect("registered task must start");

        drop(registry);
        tokio::time::timeout(Duration::from_secs(1), async {
            while !dropped.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("aborted task must drop its state");
    }
}
