use crate::ctx::Ctx;
use crate::error::Result;

/// A long-running background task owned by the application lifecycle.
///
/// Tasks start when [`crate::App::run`] starts and are cancelled cooperatively on
/// shutdown. A task's `run` receives a detached [`Ctx`] whose cancellation token
/// fires on shutdown; every task must select against it in its loop.
///
/// Scheduled work is just a task that drives a `tokio::time::interval`. Hephos does
/// not ship a cron engine — the canonical pattern lives in `examples/`.
///
/// ```ignore
/// struct OutboxDrain;
///
/// impl Task<AppState> for OutboxDrain {
///     fn name(&self) -> &'static str { "outbox_drain" }
///
///     async fn run(&self, ctx: Ctx<AppState>) -> hephos::Result<()> {
///         let mut tick = tokio::time::interval(Duration::from_secs(5));
///         loop {
///             tokio::select! {
///                 _ = ctx.cancel_token().cancelled() => break,
///                 _ = tick.tick() => ctx.state().outbox.drain().await?,
///             }
///         }
///         Ok(())
///     }
/// }
/// ```
pub trait Task<S>: Send + Sync + 'static {
    fn name(&self) -> &'static str;

    fn run(&self, ctx: Ctx<S>) -> impl std::future::Future<Output = Result<()>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    struct TestTask {
        executed: Arc<AtomicBool>,
    }

    impl Task<()> for TestTask {
        fn name(&self) -> &'static str {
            "test_task"
        }

        async fn run(&self, ctx: Ctx<()>) -> Result<()> {
            tokio::select! {
                _ = ctx.cancel_token().cancelled() => {
                    self.executed.store(true, Ordering::SeqCst);
                }
                _ = tokio::time::sleep(Duration::from_secs(10)) => {}
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn task_runs_and_cancels_cooperatively() {
        let executed = Arc::new(AtomicBool::new(false));
        let task = TestTask {
            executed: executed.clone(),
        };
        assert_eq!(task.name(), "test_task");

        let cancel = CancellationToken::new();
        let ctx = Ctx::detached(()).with_cancel(cancel.clone());

        let handle = tokio::spawn(async move {
            task.run(ctx).await.unwrap();
        });

        cancel.cancel();
        handle.await.unwrap();

        assert!(executed.load(Ordering::SeqCst));
    }

    struct FailingTask;

    impl Task<()> for FailingTask {
        fn name(&self) -> &'static str {
            "failing_task"
        }

        async fn run(&self, _ctx: Ctx<()>) -> Result<()> {
            Err(crate::Error::internal("task failure"))
        }
    }

    #[tokio::test]
    async fn task_propagates_error() {
        let task = FailingTask;
        assert_eq!(task.name(), "failing_task");
        let ctx = Ctx::detached(());
        let res = task.run(ctx).await;
        assert!(res.is_err());
    }
}
