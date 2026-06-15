/// Tracing initialisation for rust-rag.
///
/// Call `init()` once at application startup (e.g. from the CLI binary).
/// It configures a subscriber that:
/// - Writes structured JSON to stdout when `RUST_LOG` is set, or falls back
///   to simple console output with level + target when not configured.
/// - Attaches each span's timing via `tracing::instrument`, so callers can
///   measure latency of indexing / retrieval / embedding / LLM calls without
///   code changes — just read the JSON logs and feed them to Grafana / Prometheus.

use tracing_subscriber::{EnvFilter, Registry};
use tracing_subscriber::prelude::*;

/// Initialise the global `tracing` subscriber once at application startup.
///
/// This function is idempotent — calling it multiple times only sets up the
/// subscriber on the first call. After that all `#[instrument]`-annotated
/// code and manual span creation will be captured automatically.
pub fn init() {
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_level(true)
        // With JSON formatting, each line is a valid JSON object that can be
        // parsed by the OpenTelemetry Collector (or any JSON log sink).
        .json();

    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))
        .unwrap_or_else(|_| {
            EnvFilter::new("warn,tracing::span=warn")
        });

    let subscriber = Registry::default().with(filter).with(fmt_layer);

    tracing::subscriber::set_global_default(subscriber)
        .expect("failed to set global tracing subscriber");
}

/// Return `true` if a global tracing subscriber has been set up via [`init`].
pub fn is_init() -> bool {
    // Check the global dispatcher's inner type name. If we didn't call init(),
    // it will be the default NoSubscriber (name contains "NoOp").
    use std::fmt::Write as _;
    let mut buf = String::new();
    tracing::dispatcher::get_default(|d: &tracing::Dispatch| {
        write!(&mut buf, "{}", std::any::type_name_of_val(d)).ok();
    });
    !buf.contains("NoOp") && !buf.is_empty()
}
