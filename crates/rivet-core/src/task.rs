use crate::ctx::Ctx;
use crate::error::Result;

/// A long-running background task owned by the application lifecycle.
///
/// Tasks start when [`crate::App::run`] starts and are cancelled cooperatively on
/// shutdown. A task's `run` receives a detached [`Ctx`] whose cancellation token
/// fires on shutdown; every task must select against it in its loop.
///
/// Scheduled work is just a task that drives a `tokio::time::interval`. Rivet does
/// not ship a cron engine — the canonical pattern lives in `examples/`.
///
/// ```ignore
/// struct OutboxDrain;
///
/// impl Task<AppState> for OutboxDrain {
///     fn name(&self) -> &'static str { "outbox_drain" }
///
///     async fn run(&self, ctx: Ctx<AppState>) -> rivet::Result<()> {
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
