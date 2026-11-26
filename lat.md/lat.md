# observability

Two-crate observability lib, reusable by any Rust project. [[crates/observability-core/src/lib.rs]] is the hot-path metrics core; [[crates/observability/src/lib.rs]] wires tracing and metrics backends on top of it.

`observability-core` depends only on `metrics` + `hdrhistogram` (+ optional `tokio` flusher), so a hot-path recv loop can pull it without dragging in `opentelemetry` or `tracing-subscriber`. All backend wiring lives one crate up, in `observability`.

## Two-subsystem split

[[crates/observability/src/lib.rs#init]] brings up two subsystems that share nothing at runtime except one `OtlpConfig`.

[[crates/observability/src/pipeline/mod.rs#init_pipeline]] builds a single `tracing_subscriber::Registry`: a logging fan-out over every configured sink (stdout, rolling file, OTLP-log bridge), each with its own format and level filter, fused with exactly one tracing OTLP span sink. Tracing is never fanned out and never written to disk, by design; only logging fans out.

`init_pipeline` builds every layer in `build_layers` before `try_init`, so a config the build cannot honor returns `Err` with no subscriber set: `FeatureDisabled` for a missing cargo feature, `MissingOtlp` for tracing or an OTLP log sink without `otlp`, and `Exporter` (with the `Signal`) when the export runtime or an exporter fails to build. The crate prints nothing to stderr. One logger provider serves every OTLP log sink, and each sink gets its own bridge layer at its own level.

A rolling file sink whose directory cannot be created or opened returns `PipelineError::RollingFile` with the directory. `max_files` caps the files with the sink's prefix; `tracing-appender` prunes the oldest when the sink opens and on each rotation, so a restart enforces the cap too.

Every layer in the registry carries a filter, because one unfiltered layer makes the registry report no max level hint and lifts the global max level to `trace`. The OTel span layer and the OTLP log bridge use [[crates/observability/src/pipeline/filter.rs#otlp_filter]]: the sink's `EnvFilter` combined with [[crates/observability/src/pipeline/filter.rs#ExporterThreadFilter]], which drops events and spans created on the export runtime's threads and the SDK's batch and periodic threads, so exporting never produces more telemetry for the same exporter. The fmt sinks keep only their level filter, so export errors still show in local logs. The bunyan span storage takes the filter of the most verbose bunyan sink.

`ObsGuard`, [[crates/observability/src/pipeline/guard.rs#PipelineGuard]], and `MetricsGuard` are `#[must_use]`. A statement like `init_pipeline(cfg)?;` drops the guard at once, which stops the log writers and turns the metrics gate off, so the lint makes that mistake a compile-time warning.

[[crates/observability/src/metrics.rs#init_metrics]] installs the `metrics`-facade global recorder (Prometheus pull or OTLP push) and flips the `observability-core` gate on success only; any error path leaves the gate untouched. It runs independently of the pipeline: `init` calls `init_pipeline` first (so a metrics failure is itself logged), and a metrics install failure downgrades to a `tracing::warn!` plus `metrics: None`, never taking down logging or tracing.

The only runtime coupling between the two subsystems is the `OtlpConfig` value `init` clones out of `PipelineConfig` before calling `init_pipeline`, then hands to `init_metrics` alongside the service name.

## Hot-path memory model (observability-core)

`observability-core` never touches a shared atomic on the hot path once a thread has primed its gate mirror. Everything sampled per-message lives in thread-local state, merged into shared registries only on a periodic tick.

The runtime gate is a process-wide [[crates/observability-core/src/lib.rs#METRICS_ON]] `AtomicBool`, mirrored per-thread into a `Cell<Option<bool>>` refreshed via `refresh_thread_gate`. The hot-path check reads the thread-local mirror first and only falls back to the shared atomic before that mirror has ever been primed on the calling thread.

Latency samples land in a thread-local bounded `hdrhistogram::Histogram` (1ns..60s, 3 sig figs, `saturating_record` so a tail spike clamps into range instead of panicking) via [[crates/observability-core/src/lib.rs#record_latency_ns]]. [[crates/observability-core/src/lib.rs#merge_local]] hands that thread's histogram, plus its message count, into a registry-backed shared slot: one slot per thread that has ever merged. [[crates/observability-core/src/lib.rs#drain_all]] folds every registered slot into one snapshot and clears each slot in place, so a flusher on another thread only ever sees data a worker thread has already merged, never a worker's live local state.

Message counting mirrors the same shape: a plain per-thread `Cell<u64>` on the hot path (`count_msg`/`take_count`), never a shared atomic, drained into the shared registry alongside the histogram on the same `merge_local` tick.

[[crates/observability-core/src/lib.rs#should_sample]] is a per-thread wraparound sampler (`mask` must be `2^n - 1`, e.g. `SAMPLE_1_IN_8192`) for hot-path call sites that need periodic-not-every-message sampling. It samples true on a fresh thread's very first call, then exactly once per `mask + 1` consecutive calls after that.

## Shared OTLP, three sibling exporters

[[crates/observability/src/otlp/mod.rs#OtlpConfig]] is the single source of endpoint, protocol, headers, resource attrs, and timeout for every OTLP signal.

Its `Debug` output prints each header name with `[REDACTED]` in place of the value, because header values usually carry API keys. `PipelineConfig` derives `Debug` and embeds the `OtlpConfig`, so `?cfg` in a log line is safe for both.

`pipeline/otel.rs` and `metrics.rs` each call its exporter builders directly: `span_exporter` for tracing, `metric_exporter` (under `metrics-otel`) for the metrics bridge, `log_exporter` (under `otel-logs`) for the log bridge layer, all off the same endpoint/timeout/headers pair. No `opentelemetry` type crosses into a public `observability` signature; the builders stay `pub(crate)` and otel-gated.

[[crates/observability/src/otlp/runtime.rs#ExportRuntime]] is the one tokio runtime (multi-thread, 1 worker, threads named `observability-otlp`) that runs every exporter's I/O, so OTLP export works with or without an app runtime. gRPC exporters are built inside its context, so tonic spawns its channel worker there. HTTP exporters send through [[crates/observability/src/otlp/http_client.rs#RuntimeHttpClient]], an async reqwest client whose requests are spawned onto that runtime.

Dropping a guard shuts its OTel providers down, which exports the final batch, and logs a failed shutdown as a warning. Export runs on the export runtime, so this works from a thread with no runtime and from inside a current_thread or multi-thread app runtime, async context included. `ObsGuard` declares `metrics` before `pipeline`, so the metrics recorder shuts down first and the log sinks still print its warnings.

`ExportRuntime::shared` hands out the live runtime through a `Weak` slot, so the pipeline and metrics guards share one export thread, and an init after every guard dropped starts a fresh one. Each guard holds its `Arc` behind its providers, so the runtime outlives their final flush. The last `Arc` to drop calls `shutdown_background`, which is safe inside an async context.

Every configured header goes out on both transports: as HTTP headers, or as gRPC metadata through `MetadataMap::from_headers`. A header whose name or value is not valid HTTP returns `OtlpError::InvalidHeader` at init, naming the header and never its value, because opentelemetry-otlp would drop it without a word.

TLS applies only to `https://` endpoints, so `http://localhost` stays plain text. [[crates/observability/src/otlp/tls.rs#OtlpTls]] picks the roots, and the default `tls` feature turns it on. `TlsRoots::Native` is the OS store, checked for at least one certificate at init so both transports fail the same way when it is empty. `TlsRoots::Webpki` is the bundled Mozilla set under `tls-webpki-roots`. `TlsRoots::CaFileOnly` trusts only `ca_file`. `ca_file` adds its CA certificates to any roots choice, and `client_cert` presents a certificate for mutual TLS. Both files are read and parsed at init, so a missing or malformed file returns `OtlpError::TlsFile` or `OtlpError::InvalidPem` with its path on either transport. Without `tls`, an `https://` endpoint returns `OtlpError::FeatureDisabled`. ring is the crypto provider unless the app installs a process-wide one first.

`otel_resource` folds `service.name` plus every configured `resource` attribute onto whichever provider calls it, so span, metric, and log all carry identical resource attributes from one config, with no per-signal drift.

## Feature model

`otel` is the grouping feature. Per-signal features layer on top of it.

`otel` pulls in `opentelemetry`/`opentelemetry_sdk`/`opentelemetry-otlp`/`tracing-opentelemetry` and gates every `#[cfg(feature = "otel")]` block. `otel-traces` is a plain alias for `otel`. `otel-logs` adds the OTLP log bridge (`opentelemetry-appender-tracing`). `metrics-prom` adds the Prometheus pull exporter. `metrics-otel` adds the OTLP metrics bridge and implies `otel`. `bunyan` adds the bunyan-format log sink.

No feature is named `tracing` or `metrics`; those names stay reserved for the extern facade crate deps themselves, so a feature flag never shadows `tracing::` or `metrics::` path resolution.

The whole otel stack pins to opentelemetry 0.31: `metrics-exporter-otel` 0.3.1 pins `opentelemetry ^0.31`, and all three OTLP signal exporters plus the metrics bridge must resolve to one `opentelemetry` version, so that crate is the version bottleneck for the entire stack.

## See also

Related sections outside this file:

- [[tests]] documents test specifications for the runtime gate, sampler, per-thread counters, histogram merge, and OTel field keys.
