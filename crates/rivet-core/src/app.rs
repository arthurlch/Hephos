use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::signal;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::config::{Config, LogFormat};
use crate::ctx::{Ctx, RequestId};
use crate::error::{Error, Result};
use crate::identity::Identity;
use crate::router::Router;
use crate::task::Task;

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
/// async fn main() -> rivet::Result<()> {
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
            let ctx = Ctx::detached(state.clone());
            let token = shutdown.clone();
            let ctx = ctx.with_cancel(token);
            handles.push(tokio::spawn(async move {
                if let Err(error) = task.run_erased(ctx).await {
                    tracing::error!(task = task.name(), error = %error, "task failed");
                }
            }));
        }

        let app = self
            .router
            .into_inner()
            .layer(standard_stack(shutdown.clone()))
            .with_state(state);

        let listener = TcpListener::bind(config.addr)
            .await
            .map_err(|e| Error::internal(format!("bind {}: {e}", config.addr)))?;
        tracing::info!(addr = %config.addr, "rivet listening");

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

        for handle in handles {
            let _ = handle.await;
        }
        Ok(())
    }
}

/// The standard middleware stack, in fixed order, identical in every app. This
/// is the only place cross-cutting HTTP behavior is configured.
///
/// BOUNDARY: shown as the intended composition. The concrete tower layers
/// (request-id injection, tracing span, timeout, body limit, cancellation token
/// propagation) are assembled here so application code never touches tower.
fn standard_stack<S>(_shutdown: CancellationToken) -> tower::layer::util::Identity {
    // BOUNDARY: replace `Identity` with the composed tower stack:
    //   RequestIdLayer -> TraceLayer -> TimeoutLayer -> RequestBodyLimitLayer
    //   -> a layer that inserts a per-request CancellationToken (child of
    //      `_shutdown`) and an `Identity` default into request extensions.
    // Kept as `Identity` here so the crate's public shape compiles without the
    // full wiring; see ARCHITECTURE.md § Request lifecycle.
    tower::layer::util::Identity::new()
}

fn init_tracing(format: LogFormat) {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

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

// Internal constructor used by `App::run` to build the per-request context seed.
// Kept here to document the extension types the middleware stack must insert.
#[allow(dead_code)]
fn seed_extensions() -> (Identity, RequestId, CancellationToken) {
    (
        Identity::Anonymous,
        RequestId(Uuid::new_v4()),
        CancellationToken::new(),
    )
}

#[allow(dead_code)]
type SharedState<S> = Arc<S>;
