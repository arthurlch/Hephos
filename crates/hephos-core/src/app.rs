use std::time::Duration;

use tokio::net::TcpListener;
use tokio::signal;
use tokio_util::sync::CancellationToken;

use crate::config::{Config, LogFormat};
use crate::ctx::Ctx;
use crate::error::{Error, Result};
use crate::router::Router;
use crate::stack::{self, StackConfig};
use crate::task::Task;

/// How long to wait for background tasks to finish after shutdown is signalled
/// before giving up on them.
const DRAIN_DEADLINE: Duration = Duration::from_secs(10);

/// The application. One per process. Built once in `main`, then `run`.
///
/// `App` owns the composition root: state, routes, and background tasks. It reads
/// [`Config`] from the environment, installs the standard middleware stack
/// (tracing span + request id, timeout, body limit, cancellation), binds the
/// listener, starts tasks, serves, and shuts everything down gracefully on
/// SIGINT/SIGTERM.
///
/// ```ignore
/// #[tokio::main]
/// async fn main() -> hephos::Result<()> {
///     let state = AppState::init().await?;
///     App::new(state)
///         .routes(api::routes())
///         .task(OutboxDrain)
///         .run()
///         .await
/// }
/// ```
pub struct App<S> {
    state: S,
    router: Router<S>,
    tasks: Vec<Box<dyn ErasedTask<S>>>,
}

impl<S> App<S>
where
    S: Clone + Send + Sync + 'static,
{
    pub fn new(state: S) -> Self {
        App {
            state,
            router: Router::new(),
            tasks: Vec::new(),
        }
    }

    /// Install the application's routes. Called once; the router is built by the
    /// app's `api::routes()` function.
    pub fn routes(mut self, router: Router<S>) -> Self {
        self.router = router;
        self
    }

    /// Register a background task. Tasks run for the lifetime of the process and
    /// are cancelled on shutdown.
    pub fn task<T: Task<S>>(mut self, task: T) -> Self {
        self.tasks.push(Box::new(task));
        self
    }

    /// Read config, initialize observability, start tasks, and serve until a
    /// shutdown signal, then drain in-flight requests and tasks.
    pub async fn run(self) -> Result<()> {
        let config = Config::from_env()?;
        init_tracing(config.log);

        let shutdown = CancellationToken::new();
        let state = self.state;

        // Background tasks share the app's shutdown token.
        let mut handles = Vec::new();
        for task in self.tasks {
            let ctx = Ctx::detached(state.clone()).with_cancel(shutdown.clone());
            let name = task.name();
            handles.push(tokio::spawn(async move {
                if let Err(error) = task.run_erased(ctx).await {
                    tracing::error!(task = name, error = %error, "task failed");
                }
            }));
        }

        let stack_config = StackConfig {
            timeout: config.timeout,
            body_limit: config.body_limit,
        };
        let app = stack::apply(self.router.into_inner(), stack_config, shutdown.clone())
            .with_state(state);

        let listener = TcpListener::bind(config.addr)
            .await
            .map_err(|e| Error::internal(format!("bind {}: {e}", config.addr)))?;
        tracing::info!(addr = %config.addr, "hephos listening");

        let serve = axum::serve(listener, app).with_graceful_shutdown({
            let token = shutdown.clone();
            async move {
                wait_for_signal().await;
                tracing::info!("shutdown signal received");
                token.cancel();
            }
        });

        serve
            .await
            .map_err(|e| Error::internal(format!("serve: {e}")))?;

        // Tasks were cancelled by the shutdown token; drain them with a deadline so a
        // stuck task cannot wedge shutdown, and surface panics rather than swallowing.
        for handle in handles {
            match tokio::time::timeout(DRAIN_DEADLINE, handle).await {
                Ok(Ok(())) => {}
                Ok(Err(join_error)) => {
                    tracing::error!(error = %join_error, "background task panicked");
                }
                Err(_) => {
                    tracing::warn!("background task did not finish within the drain deadline");
                }
            }
        }
        Ok(())
    }
}

fn init_tracing(format: LogFormat) {
    use tracing_subscriber::{EnvFilter, fmt, prelude::*};

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let registry = tracing_subscriber::registry().with(filter);
    match format {
        LogFormat::Json => registry.with(fmt::layer().json()).init(),
        LogFormat::Pretty => registry.with(fmt::layer()).init(),
    }
}

async fn wait_for_signal() {
    let ctrl_c = async {
        let _ = signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) = signal::unix::signal(signal::unix::SignalKind::terminate()) {
            sig.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

// Object-safe shim so tasks of different concrete types can share one Vec.
trait ErasedTask<S>: Send + Sync {
    fn name(&self) -> &'static str;
    fn run_erased(
        self: Box<Self>,
        ctx: Ctx<S>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>;
}

impl<S, T> ErasedTask<S> for T
where
    S: Clone + Send + Sync + 'static,
    T: Task<S>,
{
    fn name(&self) -> &'static str {
        Task::name(self)
    }

    fn run_erased(
        self: Box<Self>,
        ctx: Ctx<S>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>> {
        Box::pin(async move { self.run(ctx).await })
    }
}
