//! OpenTelemetry integration and RED metrics (request rate, errors, duration).
//!
//! Provides canonical metrics collection labeled by route template (`/users/{id}`),
//! preventing label cardinality explosion, and OTLP export for logs, metrics, and traces.

use std::time::Instant;

use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;

use crate::config::LogFormat;
use crate::error::Result;

/// OpenTelemetry configuration for remote export.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TelemetryConfig {
    /// OTLP exporter endpoint (e.g. `http://localhost:4318`).
    pub otlp_endpoint: Option<String>,
    /// Whether distributed tracing export is enabled (traces are gated).
    pub traces_enabled: bool,
}

/// Handles to OpenTelemetry SDK providers for flushing during graceful shutdown.
pub struct TelemetryHandles {
    meter_provider: Option<SdkMeterProvider>,
    logger_provider: Option<SdkLoggerProvider>,
    tracer_provider: Option<SdkTracerProvider>,
}

impl TelemetryHandles {
    /// Construct empty handles (when remote telemetry export is disabled).
    pub fn empty() -> Self {
        Self {
            meter_provider: None,
            logger_provider: None,
            tracer_provider: None,
        }
    }

    /// Flush and shutdown all initialized OpenTelemetry providers.
    pub fn shutdown(self) {
        if let Some(mp) = self.meter_provider {
            let _ = mp.shutdown();
        }
        if let Some(lp) = self.logger_provider {
            let _ = lp.shutdown();
        }
        if let Some(tp) = self.tracer_provider {
            let _ = tp.shutdown();
        }
    }
}

use std::sync::Arc;

/// Hook callback type for observing recorded RED metrics (useful in tests).
pub type RedMetricsHook = Arc<dyn Fn(&str, &str, u16, f64) + Send + Sync>;

/// RED metrics instruments (Rate, Errors, Duration) for HTTP requests.
#[derive(Clone)]
pub struct RedMetrics {
    requests: Counter<u64>,
    duration: Histogram<f64>,
    errors: Counter<u64>,
    hook: Option<RedMetricsHook>,
}

impl std::fmt::Debug for RedMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedMetrics")
            .field("requests", &self.requests)
            .field("duration", &self.duration)
            .field("errors", &self.errors)
            .field("has_hook", &self.hook.is_some())
            .finish()
    }
}

impl Default for RedMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl RedMetrics {
    /// Construct a new RED metrics collector using the global meter.
    pub fn new() -> Self {
        let meter = opentelemetry::global::meter("hephos");
        let requests = meter
            .u64_counter("http.server.requests")
            .with_description("Total count of HTTP requests processed")
            .with_unit("{request}")
            .build();
        let duration = meter
            .f64_histogram("http.server.request.duration")
            .with_description("Duration of HTTP request execution in seconds")
            .with_unit("s")
            .build();
        let errors = meter
            .u64_counter("http.server.errors")
            .with_description("Total count of HTTP server error responses (5xx)")
            .with_unit("{error}")
            .build();

        Self {
            requests,
            duration,
            errors,
            hook: None,
        }
    }

    /// Register a hook invoked whenever a request metric is recorded.
    pub fn with_hook(
        mut self,
        hook: impl Fn(&str, &str, u16, f64) + Send + Sync + 'static,
    ) -> Self {
        self.hook = Some(Arc::new(hook));
        self
    }

    /// Record request execution.
    ///
    /// Labels are strictly constrained to:
    /// - `http.request.method`: the HTTP method (e.g. GET, POST)
    /// - `http.route`: the matched route template (e.g. `/users/{id}`) or `"__unmatched__"`
    /// - `http.response.status_code`: the HTTP response status code as string
    pub fn record(&self, method: &str, route: &str, status: u16, duration_secs: f64) {
        if let Some(hook) = &self.hook {
            hook(method, route, status, duration_secs);
        }

        let attributes = [
            KeyValue::new("http.request.method", method.to_string()),
            KeyValue::new("http.route", route.to_string()),
            KeyValue::new("http.response.status_code", status.to_string()),
        ];

        self.requests.add(1, &attributes);
        self.duration.record(duration_secs, &attributes);
        if status >= 500 {
            self.errors.add(1, &attributes);
        }
    }
}

/// Initialize tracing subscriber and OpenTelemetry providers.
pub fn init_telemetry(format: LogFormat, config: &TelemetryConfig) -> Result<TelemetryHandles> {
    use tracing_subscriber::{EnvFilter, fmt, prelude::*};

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let mut meter_provider_handle = None;
    let mut logger_provider_handle = None;
    let mut tracer_provider_handle = None;

    if let Some(endpoint) = &config.otlp_endpoint {
        let metrics_endpoint = signal_endpoint(endpoint, "v1/metrics");
        let metric_exporter = opentelemetry_otlp::MetricExporter::builder()
            .with_http()
            .with_endpoint(metrics_endpoint)
            .build()
            .map_err(|e| {
                crate::Error::internal(format!("failed to build OTLP metric exporter: {e}"))
            })?;

        let meter_provider = SdkMeterProvider::builder()
            .with_periodic_exporter(metric_exporter)
            .build();
        opentelemetry::global::set_meter_provider(meter_provider.clone());
        meter_provider_handle = Some(meter_provider);

        // 2. Logs exporter (stable upstream, on by default)
        let log_exporter = opentelemetry_otlp::LogExporter::builder()
            .with_http()
            .with_endpoint(signal_endpoint(endpoint, "v1/logs"))
            .build()
            .map_err(|e| {
                crate::Error::internal(format!("failed to build OTLP log exporter: {e}"))
            })?;

        let logger_provider = SdkLoggerProvider::builder()
            .with_batch_exporter(log_exporter)
            .build();
        logger_provider_handle = Some(logger_provider);

        // 3. Traces exporter (beta upstream, gated behind traces_enabled flag)
        if config.traces_enabled {
            let span_exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                .with_endpoint(signal_endpoint(endpoint, "v1/traces"))
                .build()
                .map_err(|e| {
                    crate::Error::internal(format!("failed to build OTLP span exporter: {e}"))
                })?;

            let tracer_provider = SdkTracerProvider::builder()
                .with_batch_exporter(span_exporter)
                .build();
            opentelemetry::global::set_tracer_provider(tracer_provider.clone());
            tracer_provider_handle = Some(tracer_provider);
        }
    }

    // Build tracing-subscriber registry
    let registry = tracing_subscriber::registry().with(filter);

    match format {
        LogFormat::Json => {
            let otel_log_layer = logger_provider_handle
                .as_ref()
                .map(opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new);
            let otel_trace_layer = if config.traces_enabled && tracer_provider_handle.is_some() {
                Some(tracing_opentelemetry::OpenTelemetryLayer::new(
                    opentelemetry::global::tracer("hephos"),
                ))
            } else {
                None
            };
            let _ = registry
                .with(fmt::layer().json())
                .with(otel_log_layer)
                .with(otel_trace_layer)
                .try_init();
        }
        LogFormat::Pretty => {
            let otel_log_layer = logger_provider_handle
                .as_ref()
                .map(opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new);
            let otel_trace_layer = if config.traces_enabled && tracer_provider_handle.is_some() {
                Some(tracing_opentelemetry::OpenTelemetryLayer::new(
                    opentelemetry::global::tracer("hephos"),
                ))
            } else {
                None
            };
            let _ = registry
                .with(fmt::layer())
                .with(otel_log_layer)
                .with(otel_trace_layer)
                .try_init();
        }
    }

    Ok(TelemetryHandles {
        meter_provider: meter_provider_handle,
        logger_provider: logger_provider_handle,
        tracer_provider: tracer_provider_handle,
    })
}

fn signal_endpoint(base: &str, signal_path: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with("/v1/metrics")
        || trimmed.ends_with("/v1/logs")
        || trimmed.ends_with("/v1/traces")
    {
        trimmed.to_string()
    } else {
        format!("{trimmed}/{signal_path}")
    }
}

/// Route-level middleware to record RED metrics with the matched route template.
pub async fn red_metrics_middleware(metrics: RedMetrics, req: Request, next: Next) -> Response {
    let start = Instant::now();
    let method = req.method().as_str().to_string();
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| "__unmatched__".to_string());

    let response = next.run(req).await;
    let duration = start.elapsed().as_secs_f64();
    let status = response.status().as_u16();

    metrics.record(&method, &route, status, duration);

    response
}

/// Fallback handler for unmatched routes so 404s are recorded with a bounded label.
pub async fn unmatched_route_fallback(metrics: RedMetrics, req: Request) -> Response {
    let method = req.method().as_str().to_string();
    let duration = 0.0;
    let status = 404;

    metrics.record(&method, "__unmatched__", status, duration);
    crate::Error::not_found("not found").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn red_metrics_records_without_panic() {
        let metrics = RedMetrics::new();
        metrics.record("GET", "/users/{id}", 200, 0.015);
        metrics.record("POST", "/users", 201, 0.030);
        metrics.record("GET", "__unmatched__", 404, 0.001);
        metrics.record("GET", "/crash", 500, 0.005);
    }

    #[test]
    fn empty_handles_shutdown_cleanly() {
        let handles = TelemetryHandles::empty();
        handles.shutdown();
    }

    #[tokio::test]
    async fn matched_path_extraction_in_route_layer() {
        use axum::routing::get;
        use tower::ServiceExt;

        let router = axum::Router::new()
            .route("/users/{id}", get(|| async { "ok" }))
            .route_layer(axum::middleware::from_fn(
                |req: Request, next: Next| async move {
                    let mp = req
                        .extensions()
                        .get::<MatchedPath>()
                        .map(|p| p.as_str().to_string());
                    let mut res = next.run(req).await;
                    if let Some(mp) = mp {
                        res.headers_mut()
                            .insert("x-matched-path", mp.parse().unwrap());
                    }
                    res
                },
            ));

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/users/42")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.headers().get("x-matched-path").unwrap(),
            "/users/{id}"
        );
    }

    #[tokio::test]
    async fn matched_path_extraction_in_outer_layer() {
        use axum::routing::get;
        use tower::ServiceExt;

        let router = axum::Router::new()
            .route("/users/{id}", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(
                |req: Request, next: Next| async move {
                    let mp = req
                        .extensions()
                        .get::<MatchedPath>()
                        .map(|p| p.as_str().to_string());
                    let mut res = next.run(req).await;
                    res.headers_mut().insert(
                        "x-matched-path",
                        mp.unwrap_or_else(|| "__unmatched__".to_string())
                            .parse()
                            .unwrap(),
                    );
                    res
                },
            ));

        // 1. Matched route
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/users/42")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.headers().get("x-matched-path").unwrap(),
            "/users/{id}"
        );

        // 2. Unmatched route (404)
        let response_404 = router
            .oneshot(
                Request::builder()
                    .uri("/not-found")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response_404.status(), 404);
        assert_eq!(
            response_404.headers().get("x-matched-path").unwrap(),
            "__unmatched__"
        );
    }

    #[tokio::test]
    async fn matched_path_preserved_across_merged_routers() {
        use axum::routing::get;
        use tower::ServiceExt;

        let app_router = axum::Router::new().route("/items/{id}", get(|| async { "item" }));
        let probe_router = axum::Router::new().route("/health", get(|| async { "ok" }));

        let router = app_router
            .merge(probe_router)
            .layer(axum::middleware::from_fn(
                |req: Request, next: Next| async move {
                    let mp = req
                        .extensions()
                        .get::<MatchedPath>()
                        .map(|p| p.as_str().to_string());
                    let mut res = next.run(req).await;
                    res.headers_mut().insert(
                        "x-matched-path",
                        mp.unwrap_or_else(|| "__unmatched__".to_string())
                            .parse()
                            .unwrap(),
                    );
                    res
                },
            ));

        let res_item = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/items/99")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res_item.headers().get("x-matched-path").unwrap(),
            "/items/{id}"
        );

        let res_health = router
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res_health.headers().get("x-matched-path").unwrap(),
            "/health"
        );
    }
}
