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

<!-- x-release-please-start-version -->
```toml
[dependencies]
observability = { git = "https://github.com/0x67/observability", tag = "observability-v0.3.1" }
```
<!-- x-release-please-end -->

Using both crates? Pin both to the same tag, the newest one of either component. Two tags
on one git url check out two copies of the repository, and `observability-core` types from
one copy do not match the other.

```rust,no_run
use observability::{MetricsConfig, ObsConfig, OtlpConfig, PipelineConfig, TracingConfig, init};

fn main() -> Result<(), observability::PipelineError> {
    // Hold the guard for the whole process. Drop shuts the metrics recorder down,
    // then exports the final OTLP batches and flushes the log writers.
    let _guard = init(ObsConfig {
        pipeline: PipelineConfig {
            service_name: "feed-handler".into(),
            otlp: Some(OtlpConfig {
                endpoint: "http://collector:4318".into(),
                ..Default::default()
            }),
            tracing: Some(TracingConfig::default()),
            ..Default::default()
        },
        metrics: Some(MetricsConfig::default()),
    })?;
    tracing::info!("observability up");
    Ok(())
}
```

Each config type implements `Default`, and a setup sets only the fields that differ. This one
logs JSON to stdout at `info`, exports spans over OTLP HTTP/protobuf, and serves Prometheus
metrics on `127.0.0.1:9000`. `endpoint` is the collector base URL: HTTP export appends `/v1/traces`,
`/v1/logs`, or `/v1/metrics`. With the `serde` feature the same config reads from a TOML, YAML,
or JSON file; the [`observability` crate README](crates/observability#config-files) has the
format.

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

## Compatibility

Each tag builds against one version of each upstream crate below. An app that also uses
`opentelemetry` or `metrics` directly needs the same minor version: with a second version,
Cargo builds two copies, and each copy has a separate global tracer provider or metrics
recorder.

| observability tag | observability-core | opentelemetry family | tracing-opentelemetry | metrics-exporter-otel | metrics | metrics-exporter-prometheus | tracing / tracing-subscriber | tonic | MSRV |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| observability-v0.2.0 | 0.2.0 | 0.31 | 0.32 | 0.3 | 0.24 | 0.17 | 0.1 / 0.3 | 0.14 | 1.96.1 |
| observability-v0.2.1 | 0.2.1 | 0.31 | 0.32 | 0.3 | 0.24 | 0.17 | 0.1 / 0.3 | 0.14 | 1.96.1 |
| observability-v0.2.2 | 0.2.1 | 0.31 | 0.32 | 0.3 | 0.24 | 0.17 | 0.1 / 0.3 | 0.14 | 1.96.1 |
| observability-v0.3.0 | 0.3.0 | 0.33 | 0.34 | in-crate | 0.24 | 0.18 | 0.1 / 0.3 | 0.14 | 1.88.0 |

Upstream cells hold the major.minor version, since every crate here is 0.x and a 0.x minor
bump breaks its API. The opentelemetry family is `opentelemetry`, `opentelemetry_sdk`,
`opentelemetry-otlp`, `opentelemetry-appender-tracing`, `opentelemetry-http`, and
`opentelemetry-proto`. From 0.3.0 on, `observability` has its own OTLP metrics recorder
instead of `metrics-exporter-otel`. The `observability-core` and MSRV cells hold full versions.

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

## Operational notes

1. Keep one copy of `observability-core` in the build. An app that pins `observability` and
   `observability-core` to different tags gets two copies of the repository, and each copy of
   the core has its own metrics gate. `init_metrics` turns on the gate in the copy `observability`
   was built with. Code built against the other copy still sees `metrics_enabled()` return
   `false`, so its `count_msg` and `record_latency_ns` calls don't record anything, and no error
   says why. To check a build:

   ```bash
   cargo tree -i observability-core
   ```

   With one copy, Cargo prints the crate and what depends on it. With two, it stops with
   ``specification `observability-core` is ambiguous`` and lists both versions. Pin both crates
   to the same tag, as the [Quickstart](#quickstart) says.

2. A full `init` starts these threads, named as below:

   | Thread | Count | Started for |
   | --- | --- | --- |
   | `tracing-appender` | one per stdout or rolling-file sink | the non-blocking log writer |
   | `OpenTelemetry.Traces.BatchProcessor` | 1 | tracing |
   | `OpenTelemetry.Logs.BatchProcessor` | 1 | an OTLP log sink |
   | `OpenTelemetry.Metrics.PeriodicReader` | 1 | OTLP metrics |
   | `observability-otlp` | 1, shared by every OTLP exporter on both transports | the crate's export runtime |
   | `metrics-exporter-prometheus-http-listener` | 1 | Prometheus metrics, when no tokio runtime is current at `init`; inside one, the listener runs on the app's runtime |

   Measured on 0.3.0 on macOS, from a plain `fn main()` with no tokio runtime: stdout,
   rolling-file, and OTLP log sinks, tracing, and OTLP metrics over HTTP/protobuf to
   `http://127.0.0.1:4318` took the process from 1 thread to 7. Prometheus metrics instead of
   OTLP metrics gave the same total, with the listener in place of the periodic reader. With
   `http://localhost:4318`, a second `observability-otlp` thread appeared: tokio's blocking pool
   runs the host name lookup and stops an idle pool thread after 10 s. On Linux,
   `ls /proc/<pid>/task | wc -l` counts a process's threads and `cat /proc/<pid>/task/*/comm`
   names them, cut to 15 bytes (`OpenTelemetry.T`).

3. `std::process::exit` skips `Drop` on `ObsGuard`, and so does a panic in a binary built with
   `panic = "abort"`. Anything still inside the process is then lost: log lines queued for a
   writer, span and log batches not yet exported, and the last OTLP metrics interval. Return
   from `main` instead, with `std::process::ExitCode` if the exit code matters, or call
   `drop(guard)` before `exit`. Tauri apps drop the guard in their `RunEvent::Exit` handler (see
   the [`observability` crate README](crates/observability#two-subsystems-one-coupling)).

4. A collector or backend outage costs each signal something different:

   | Signal | Path | During the outage | After it ends |
   | --- | --- | --- | --- |
   | Metrics | Prometheus pull | The recorder keeps running totals in the process; a missed scrape loses resolution only | The next scrape returns the full totals |
   | Metrics | OTLP push | A failed export is retried, then dropped. Counters and histograms carry cumulative totals, the SDK default `Temporality::Cumulative` | The next export that succeeds carries the totals; gauge values between exports are gone |
   | Traces | OTLP | Spans wait in the batch queue, `TracingConfig::batch` (2,048 spans by default). A failed export is retried, then its batch is dropped and counted in `observability.dropped` | Spans dropped during the outage are gone |
   | Logs | OTLP sink | The same, with `LoggingConfig::otlp_batch` | Records dropped during the outage are gone |
   | Logs | Rolling file | Lines stay on disk. The writer drops a line only when its `LineQueue` is full | A collector `filelog` receiver reads the backlog from its stored offset |
   | Logs | Stdout | What the container runtime's log driver keeps | Depends on the log driver |

   The crate README's [Shared OTLP](crates/observability#shared-otlp) section has the retry
   policy, and [Delivery limits](crates/observability#delivery-limits) has the queue settings
   and the drop counts.

   For delivery across an outage, run an OTel Collector agent next to the service and point the
   crate at it. [`docs/otel-collector-agent.yaml`](docs/otel-collector-agent.yaml) keeps the
   exporter queue and the `filelog` offsets on disk through the `file_storage` extension, and
   retries failed exports:

   ```yaml
   # A collector agent next to the service: it keeps the export queue and the log file offsets on
   # disk, so queued data survives a backend outage and an agent restart.
   # Send logs one way: the OTLP log sink, or a rolling-file sink that filelog reads. With both,
   # every line arrives twice.
   extensions:
     file_storage:
       directory: /var/lib/otelcol/file_storage
       create_directory: true

   receivers:
     otlp:
       protocols:
         grpc:
           endpoint: 127.0.0.1:4317
         http:
           endpoint: 127.0.0.1:4318
     # the rolling-file sink's `dir` and `prefix`; its files are named `<prefix>.<date>`
     filelog:
       include: [/var/log/my-service/app*]
       # read what the sink wrote before the agent's first start; after that the stored offsets apply
       start_at: beginning
       storage: file_storage
       operators:
         - type: json_parser

   exporters:
     otlp:
       endpoint: backend.example.com:4317
       sending_queue:
         enabled: true
         storage: file_storage
       retry_on_failure:
         enabled: true

   service:
     extensions: [file_storage]
     pipelines:
       traces:
         receivers: [otlp]
         exporters: [otlp]
       metrics:
         receivers: [otlp]
         exporters: [otlp]
       logs:
         receivers: [otlp, filelog]
         exporters: [otlp]
   ```

## Feature model

All heavy backends are feature-gated on the `observability` crate, default-on. `otel` is the
grouping feature; per-signal features layer on top of it. No feature is named `tracing` or
`metrics`, so a flag never shadows the extern facade crates. Full matrix and the OTLP wiring
details live in the [`observability` crate README](crates/observability#feature-model).

The OTLP stack is on opentelemetry 0.33 and `tracing-opentelemetry` 0.34. The opentelemetry
crates, `tracing-opentelemetry`, `opentelemetry-http`, and `opentelemetry-proto` move as one
family, because every OTLP signal exporter and the crate's own OTLP metrics recorder share one
`opentelemetry` version. Export retries transient collector failures within
`OtlpConfig::timeout`, and the default `tls` feature uses rustls with `aws-lc-rs` on both
transports.

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

The `compat_table` test checks the last row of the [compatibility table](#compatibility)
against `Cargo.lock` and the manifests. When a pull request moves a listed crate to another
minor version or changes the MSRV, update the last row, or add a row for the next release if
the last row's tag is already out. A release pull request fails until the last row is for its
version. The test does not check the `observability-core` cell, so set it by hand to the core
version that release builds with.
