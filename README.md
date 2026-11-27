# observability

Reusable Rust observability library: a hot-path metrics core plus tracing and metrics
backend wiring on top of it. Split across two crates so a latency-sensitive receive loop
can pull the gate and helpers without dragging in `opentelemetry` or `tracing-subscriber`.

| Crate | Role | Deps |
| --- | --- | --- |
| [`observability-core`](crates/observability-core) | Runtime metrics gate + hot-path telemetry helpers. Dependency leaf. | `metrics`, `hdrhistogram`, optional `tokio` |
| [`observability`](crates/observability) | Subscriber pipeline, metrics recorder, shared OTLP config. All backend wiring. | the core + `tracing`, `tracing-subscriber`, `opentelemetry` (feature-gated) |

The rule of thumb: link `observability` in your binary or top-level service crate to bring
up logging, tracing, and metrics; link only `observability-core` in a hot-path library that
records latency and message counts but must stay free of backend crates.

## Quickstart

```toml
[dependencies]
observability = { git = "https://github.com/0x67/observability", tag = "observability-v0.2.2" }
```

Using both crates? Pin both to the same tag, the newest one of either component. Two tags
on one git url check out two copies of the repository, and `observability-core` types from
one copy do not match the other.

```rust,no_run
use std::time::Duration;
use observability::{
    init, ObsConfig, PipelineConfig, LoggingConfig, LogSink, LogSinkKind, LogFormat,
    TracingConfig, TraceExporter, MetricsConfig, MetricsExporter, OtlpConfig, OtlpProtocol,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let otlp = OtlpConfig {
        endpoint: "http://localhost:4317".into(),
        protocol: OtlpProtocol::Grpc,
        resource: [("deployment.environment".into(), "prod".into())].into(),
        timeout: Duration::from_secs(5),
        ..Default::default()
    };

    let cfg = ObsConfig {
        pipeline: PipelineConfig {
            service_name: "feed-handler".into(),
            level: "info".into(),
            otlp: Some(otlp),
            logging: Some(LoggingConfig {
                sinks: vec![LogSink {
                    kind: LogSinkKind::Stdout,
                    format: LogFormat::Json,
                    level: None,
                }],
            }),
            tracing: Some(TracingConfig { exporter: TraceExporter::Otlp }),
        },
        metrics: Some(MetricsConfig {
            exporter: MetricsExporter::Prometheus { bind: "0.0.0.0:9000".parse()? },
        }),
    };

    // Hold the guard for the whole process. Drop shuts the metrics recorder down,
    // then exports the final OTLP batches and flushes the file writers.
    let _guard = init(cfg)?;
    tracing::info!("observability up");
    Ok(())
}
```

A plain `fn main()` is enough. OTLP export runs on the crate's own `observability-otlp`
runtime, which means the app does not need a tokio runtime for export, and an app that runs
tokio does not share its executor with export. `init` returns `Err` for config the build cannot honor, such as
tracing without `otlp` or an `https://` endpoint built without the `tls` feature.

On a hot-path library that only records latency, depend on `observability-core` alone:

```rust
use observability_core::{record_latency_ns, count_msg, merge_local};

// per message on the receive loop; both are no-ops while the gate is off
let t0 = std::time::Instant::now();
// decode ...
count_msg();
record_latency_ns(t0.elapsed().as_nanos() as u64);

// on a periodic worker tick, not every message
merge_local();
```

## Architecture

`observability` brings up two subsystems that share nothing at runtime except one
`OtlpConfig`:

- **Subscriber pipeline** (`init_pipeline`): a single `tracing_subscriber::Registry`. Logging
  fans out across every configured sink (stdout, rolling file, OTLP-log bridge), each with its
  own format and level filter, fused with exactly one tracing OTLP span sink. Only logging
  fans out; tracing is never duplicated and never written to disk.
- **Metrics recorder** (`init_metrics`): the `metrics`-facade global recorder (Prometheus pull
  endpoint or OTLP push bridge). Flips the `observability-core` gate on a successful install
  only; any error path leaves the gate untouched.

`init` runs both. The pipeline comes up first so that a metrics failure is itself logged, and a
metrics install failure downgrades to a `tracing::warn!` plus `metrics: None` instead of taking
down logging or tracing. A pipeline failure is fatal and returned, and that includes config the
build cannot honor: a missing `otlp`, a missing cargo feature, an exporter that fails to build.

Both subsystems export on one tokio runtime the crate owns, so neither depends on the app's
runtime. `https://` endpoints use TLS through the default `tls` feature, with the OS store, bundled
roots, the OS platform verifier, or a private CA, and optionally a client certificate. The
[`observability` crate README](crates/observability#tls) has the details.

`observability-core` keeps hot-path writes thread-local. The runtime gate is one process-wide
atomic that every thread reads with a `Relaxed` load. The gate is written rarely, which keeps
that read in each core's cache. Latency samples land in a thread-local bounded `hdrhistogram::Histogram`, and
message counts sit in a plain per-thread `Cell`. Both move to one shared slot per thread on a
periodic tick or at thread exit. A flusher on another thread sees only data a worker already
handed off, and the first drain after a thread exits frees that thread's slot.

See [`lat.md/lat.md`](lat.md/lat.md) for the full design intent and the per-subsystem rationale.

## Feature model

All heavy backends are feature-gated on the `observability` crate, default-on. `otel` is the
grouping feature; per-signal features layer on top of it. No feature is named `tracing` or
`metrics`, so a flag never shadows the extern facade crates. Full matrix and the OTLP wiring
details live in the [`observability` crate README](crates/observability#feature-model).

The whole OTLP stack pins to opentelemetry 0.31: `metrics-exporter-otel` 0.3.1 pins
`opentelemetry ^0.31`, and every OTLP signal exporter plus the metrics bridge must resolve to
one `opentelemetry` version, so that crate gates the entire stack. Do not bump one otel crate
alone.

## Workspace

A virtual Cargo workspace (`members = ["crates/*"]`) with two members. It is one git repo,
self-sufficient when cloned alone.

- MSRV / edition: Rust 1.88.0, edition 2024 (pinned in `rust-toolchain.toml` and
  `[workspace.package]`). The MSRV is the highest `rust-version` in the locked dependency
  graph for the CI platforms, and the CI MSRV cell builds and tests on it. `resolver = "3"`
  keeps `cargo update` on versions that fit it.
- Release profile: `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`.
- Distribution: `publish = false`, consumed by git tag (`observability-v0.2.0`,
  `observability-core-v0.2.0`), not crates.io.
- License: MIT OR Apache-2.0.

## Development

```bash
cargo nextest run --workspace          # tests
cargo clippy --workspace -- -D warnings # lints
cargo hack --feature-powerset --depth 2 check   # feature-combo build guard (mirrors CI)
lat check                               # docs graph integrity
```

CI runs the shared `0x67/ci` reusable Rust workflows plus a feature-matrix job that
powersets both crates' features and asserts `observability-core` never pulls in
`opentelemetry` or `tracing-subscriber`.
