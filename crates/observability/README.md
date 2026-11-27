# observability

Tracing and metrics backend wiring over [`observability-core`](../observability-core): a
subscriber pipeline, a metrics recorder, and one shared OTLP config. This is where every heavy
backend dependency lives, all feature-gated. Depend on this crate in a binary or service crate;
depend on `observability-core` alone in a hot-path library.

## Two subsystems, one coupling

`init` brings up two subsystems that share nothing at runtime except a single `OtlpConfig`.

### Subscriber pipeline (`init_pipeline`)

Builds one `tracing_subscriber::Registry` and installs it as the global default via `try_init`
(a second call returns `PipelineError::AlreadyInit` instead of panicking).

- **Logging fans out** across every configured `LogSink`: stdout, rolling file, or the OTLP-log
  bridge, each with its own `LogFormat` (`Json`, `Bunyan`, `Pretty`) and its own level filter.
  Every non-blocking file sink hands back a `WorkerGuard` retained in `PipelineGuard`, so tail
  lines flush on shutdown.
- **Tracing is a single OTLP span sink** by design: never fanned out, never written to disk.

Each layer has a filter. The OTel span layer and the OTLP log bridge use the sink's level
filter plus a thread filter that drops events and spans created on the exporter's own threads
(`observability-otlp` and the SDK's batch and periodic threads). Exporting therefore never
produces more telemetry for the same exporter. App events with a `tonic`, `hyper`, or `reqwest`
target still export. The stdout and file sinks keep only their level filter, and export errors
still show in local logs.

`init_pipeline` builds every layer before it installs anything, and returns `Err` for config the
build cannot honor: tracing or an OTLP log sink without `otlp`, a sink that needs a cargo feature
the build lacks, an exporter that fails to build, or a log directory that cannot be created. The
crate prints nothing to stderr.

### Metrics recorder (`init_metrics`)

Installs the `metrics`-facade global recorder chosen by `MetricsExporter`:

- `Prometheus(PrometheusConfig)`: a pull endpoint, by default on `127.0.0.1:9000` with bucketed
  histograms and a `service_name` label on every series.
- `Otlp`: an OTLP push bridge over the shared `OtlpConfig`.

The `observability-core` gate flips true only on a successful install; any error path returns
before touching it. An OTLP exporter that fails to build returns `MetricsError::Exporter`.

### `init`: both at once

```rust,ignore
pub fn init(cfg: ObsConfig) -> Result<ObsGuard, PipelineError>;
```

`ObsConfig { pipeline, metrics }` where `metrics` is `Option`, so a build can run logging plus
tracing with no recorder. The pipeline comes up first (so a metrics warning is itself logged),
then metrics reuse the pipeline's cloned `OtlpConfig` and service name. A pipeline failure is
fatal and returned; a metrics failure downgrades to `tracing::warn!` plus `metrics: None`, never
taking down logging or tracing. Hold the returned `ObsGuard` for the process lifetime.

### Guard lifetime

`ObsGuard`, `PipelineGuard`, and `MetricsGuard` are `#[must_use]`: a statement such as
`init_pipeline(cfg)?;` or `let _ = init(cfg)?;` drops the guard at once, which stops the log
writers and turns the metrics gate off. Bind it, as in `let _guard = init(cfg)?;`, and keep it
until the process ends.

Dropping a guard shuts its OTel providers down, which exports the final batch, then flushes the
file writers. `ObsGuard` drops its metrics guard first, while the log sinks can still print
metric shutdown warnings. Export runs on the crate's own runtime. The drop therefore works from a
thread with no runtime and from inside a current_thread or multi-thread tokio runtime, async
context included. Inside an async context the drop blocks the calling thread for at most the
export timeout while the final batch goes out.

`process::exit` skips every `Drop`, so the final batch never leaves. Tauri apps often end that
way: drop `ObsGuard` in the `RunEvent::Exit` handler, before the process exits.

## Shared OTLP

`OtlpConfig` is the single source of endpoint, protocol (`Grpc` or `HttpProtobuf`), headers,
resource attributes, timeout, and TLS for every OTLP signal. It builds three sibling exporters,
`span_exporter`, `metric_exporter` (under `metrics-otel`), and `log_exporter` (under
`otel-logs`), off the same transport pair. No `opentelemetry` type crosses a public
`observability` signature: the builders stay `pub(crate)` and otel-gated. `otel_resource` folds
`service.name` plus every configured `resource` pair onto whichever provider calls it, so span,
metric, and log all carry identical resource attributes with no per-signal drift.

`endpoint` is the collector base URL. HTTP export drops one trailing `/` and appends the
signal's path, so `http://collector:4318` sends spans to `/v1/traces`, log records to
`/v1/logs`, and metrics to `/v1/metrics`. gRPC uses the endpoint as given, because the service
path is part of each gRPC method. An HTTP endpoint that already ends in a signal path posts to
`/v1/traces/v1/traces`; set the base URL instead. `OtlpConfig::default()` is HTTP/protobuf to
`http://localhost:4318` with a 10 s timeout.

The span, metric, and log exporters run their I/O on one tokio runtime the crate owns, with one
worker and threads named `observability-otlp`. gRPC exporters are built inside it, and HTTP exporters send through an
async `reqwest` client whose requests are spawned onto it. The app does not need a tokio runtime
for export, so a plain `fn main()` works on either transport, and an app that runs tokio does
not share its executor with export. Both the pipeline and the metrics recorder use it, and it
stops after the last guard drops.

HTTP exporters send each configured header as an HTTP header, and gRPC exporters send it as
metadata. A header whose name or value is not valid HTTP returns `OtlpError::InvalidHeader` at
init, and the error message includes the header name and never its value.
`opentelemetry-otlp` still merges `OTEL_EXPORTER_OTLP_HEADERS` on top.

An endpoint with a user name or password, such as `https://user:pass@collector:4318`, returns
`OtlpError::EndpointCredentials` at init before any other OTLP check, and the error leaves the
endpoint out. Send credentials in `headers`.

`endpoint` must start with `http://` or `https://`, in any case. Any other endpoint, including
`collector:4317` and `unix://` paths, returns `OtlpError::EndpointScheme` with the endpoint at
init. Each exporter uses the transport `protocol` names and ignores
`OTEL_EXPORTER_OTLP_PROTOCOL` and `OTEL_EXPORTER_OTLP_<SIGNAL>_PROTOCOL`.

### TLS

TLS applies only to `https://` endpoints; `http://localhost` stays plain text. `OtlpConfig::tls`
holds an `OtlpTls`:

```rust,ignore
OtlpTls {
    roots: TlsRoots::Native,          // which roots to trust
    ca_file: None,                    // PEM file of extra CA certificates
    client_cert: None,                // Some(ClientCert { cert_file, key_file }) for mutual TLS
}
```

`OtlpTls::default()` trusts the OS store. The four `TlsRoots` choices:

| `TlsRoots` | Trusts | Needs |
| --- | --- | --- |
| `Native` (default) | the OS certificate store; init fails with `OtlpError::NoNativeRoots` when it is empty | `tls` |
| `Webpki` | Mozilla roots compiled into the binary, for images with no OS CA bundle | `tls-webpki-roots` |
| `Platform` | the OS verifier: Security.framework, the Windows certificate APIs, Android's trust manager, or webpki over the OS store elsewhere; follows corporate CAs and revocation, for desktop and mobile apps such as Tauri | `tls-platform-verifier` |
| `CaFileOnly` | only the certificates in `ca_file` | `tls` |

`ca_file` adds its CA certificates to any roots choice, and is the whole trust set for
`CaFileOnly`. With `Platform` it becomes the verifier's extra roots, which Android does not
support. `client_cert` presents a PEM certificate chain (leaf first) and a PEM private key
(PKCS#8, PKCS#1, or SEC1) to collectors that require mutual TLS. Both files are read and parsed
at init: a missing file returns `OtlpError::TlsFile` and a malformed one `OtlpError::InvalidPem`,
each with the path. The config holds paths, so `Debug` output never contains key material.

On Android, `TlsRoots::Platform` reads state the app sets up through
`rustls_platform_verifier::android::init_with_env`. Call it from the app's JNI setup before
`init`. Without it, the verifier panics inside an export task: tokio reports a failed export,
no batch is delivered, and the process keeps running.

ring is the crypto provider unless the app installs a process-wide rustls provider first, for
example aws-lc for FIPS; both transports then use the app's provider. The crate does not read
`OTEL_EXPORTER_OTLP_CERTIFICATE`, `OTEL_EXPORTER_OTLP_CLIENT_CERTIFICATE`, or
`OTEL_EXPORTER_OTLP_CLIENT_KEY`; TLS comes from `OtlpConfig::tls` only. No setting skips
certificate or hostname verification.

## Redaction

Secrets get into logs, spans, and exports from two sources: the crate's own config and the
values the application records.

1. The crate redacts its own config. `OtlpConfig`'s `Debug` prints each header name with
   `[REDACTED]` in place of its value, so `?cfg` on an `OtlpConfig` or a `PipelineConfig` never
   prints an API key. An endpoint with a user name or password fails init with
   `OtlpError::EndpointCredentials`, as described under [Shared OTLP](#shared-otlp).

2. The application keeps its secrets in secret types. Hold a credential in
   `secrecy::SecretString`: its `Debug` prints `SecretBox<str>([REDACTED])`, and
   `expose_secret()` returns the value where the code needs it, such as an OTLP header.

   ```rust,ignore
   // application Cargo.toml: secrecy = "0.10"
   use std::collections::BTreeMap;

   use observability::OtlpConfig;
   use secrecy::{ExposeSecret, SecretString};

   let api_key = SecretString::from(std::env::var("BACKEND_API_KEY")?);
   tracing::info!(api_key = ?api_key, "connecting"); // prints SecretBox<str>([REDACTED])
   let otlp = OtlpConfig {
       headers: BTreeMap::from([("x-api-key".into(), api_key.expose_secret().to_owned())]),
       ..OtlpConfig::default()
   };
   ```

   For a struct that is logged whole, derive `veil::Redact` and mark each secret field with
   `#[redact(fixed = N)]`, which always prints N asterisks. A plain `#[redact]` writes `*` for
   each letter and digit but keeps the value's length and punctuation.

   ```rust,ignore
   // application Cargo.toml: veil = "0.3"
   #[derive(veil::Redact)]
   struct Login {
       user: String,
       #[redact(fixed = 8)]
       password: String,
   }
   // `?login` prints the password as ********; a plain #[redact] would keep its length and punctuation
   ```

3. An OTel Collector scrubs what the application still records, such as a token in a URL query
   or an email address in a log message.
   [`docs/otel-collector-redaction.yaml`](../../docs/otel-collector-redaction.yaml) runs three
   contrib processors in the traces and logs pipelines, ahead of `batch`:
   1. `redaction` masks the values of keys that look like credentials (password, secret,
      token, authorization, API key) and card numbers in any value, and `summary: info` adds
      counts of what it masked.
   2. `attributes/scrub` deletes `http.request.header.cookie` and replaces `enduser.id` with
      its hash.
   3. `transform/scrub` rewrites `token=` values in a span's `url.query` attribute and
      replaces email addresses in log bodies with `[EMAIL]`.

   `redaction` is first in each pipeline, and `attributes/scrub` and `transform/scrub` see the
   masked values. Use the file as is, or copy its processors into the collector config you
   already run.

4. Collector-side scrubbing runs after the data leaves the process, so unscrubbed values are sent
   over the OTLP connection to the collector. The default `tls` feature encrypts that connection
   only for an `https://` endpoint. Use `https://` unless the collector runs on the same host.

5. The crate never redacts log messages, OTLP log bodies, span names, span event names, metric
   labels, or resource attributes, so keep secrets and personal data out of them. Record
   `observability_core::field::ENDUSER_ID` on spans and logs only, never as a metric label: a
   per-user label exposes the user and grows the label set without bound.

## Feature model

`otel` is the grouping feature; per-signal features layer on top of it. No feature is named
`tracing` or `metrics`, so a flag never shadows `tracing::` or `metrics::` path resolution.

| Feature | Default | Pulls in | Effect |
| --- | :---: | --- | --- |
| `logging` | yes | none | stdout / rolling-file log sinks |
| `otel` | yes | `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-otlp`, `tracing-opentelemetry`, `tokio`, `reqwest` | gates every OTLP block; backs the tracing span sink; runs the `observability-otlp` export runtime |
| `tls` | yes | `otel` + rustls (ring) for tonic and reqwest, `rustls-native-certs` | TLS for `https://` endpoints with `TlsRoots::Native` or `CaFileOnly` |
| `tls-webpki-roots` | no | `tls` + bundled Mozilla roots | `TlsRoots::Webpki` |
| `tls-platform-verifier` | no | `tls` + `rustls-platform-verifier` | `TlsRoots::Platform` |
| `otel-traces` | no | `otel` | plain alias for `otel` |
| `otel-logs` | no | `otel` + `opentelemetry-appender-tracing` | OTLP log-bridge sink |
| `metrics-prom` | yes | `metrics-exporter-prometheus` 0.18.3 (licensed `MIT AND Apache-2.0`), `metrics-util` | Prometheus pull recorder |
| `metrics-otel` | no | `otel` + `metrics-exporter-otel` | OTLP metrics push bridge |
| `bunyan` | no | `tracing-bunyan-formatter` | bunyan-format log sink |

`default = ["logging", "otel", "tls", "metrics-prom"]`.

The whole otel stack pins to opentelemetry 0.31: `metrics-exporter-otel` 0.3.1 pins
`opentelemetry ^0.31`, and all OTLP signal exporters plus the metrics bridge must resolve to one
`opentelemetry` version, so that crate is the version bottleneck. Do not bump one otel crate
alone.

## Errors

```rust,ignore
pub enum PipelineError {
    AlreadyInit,
    MissingOtlp(Signal),                                  // tracing or an OTLP log sink without `otlp`
    FeatureDisabled { feature: &'static str },            // config needs a cargo feature the build lacks
    RollingFile { dir: PathBuf, source: Box<dyn Error + Send + Sync> },
    Exporter { signal: Signal, source: OtlpError },
    SampleRatio(f64),                                     // NaN or outside 0.0..=1.0
    DuplicateOtlpLogSink,                                 // more than one `LogSinkKind::Otlp`
}
pub enum Signal { Traces, Logs }
pub enum OtlpError {
    Runtime(io::Error),                                   // the export runtime did not start
    FeatureDisabled { feature: &'static str },            // `tls`, `tls-webpki-roots`, `tls-platform-verifier`
    NoNativeRoots,
    NoTrustAnchors,                                       // `CaFileOnly` without `ca_file`
    TlsFile { path: PathBuf, source: io::Error },
    InvalidPem { path: PathBuf, source: Box<dyn Error + Send + Sync> },
    PlatformVerifier(Box<dyn Error + Send + Sync>),
    EndpointCredentials,                                  // a user or password in `endpoint`
    EndpointScheme { endpoint: String },                  // no `http://` or `https://` prefix
    InvalidHeader { name: String },
    Build(Box<dyn Error + Send + Sync>),                  // the exporter or HTTP client did not build
}
pub enum MetricsError {
    Bind(io::Error),
    Install(Box<dyn Error + Send + Sync>),                // an empty bucket list lands here too
    Exporter(OtlpError),
}
pub enum PropagationError {                               // `otel` only
    NoOtelLayer,                                          // the pipeline has no tracing
    AlreadyStarted,                                       // set the parent before entering the span
    SpanDisabled,                                         // filters drop the span
}
```

Upstream errors are boxed, so no `opentelemetry`, `reqwest`, `tonic`, or `tracing-appender`
type appears in a public signature; the install site picks the source.

## Config surface

`PipelineConfig` and its parts are plain data with no `opentelemetry` type in any field:
`LoggingConfig` / `LogSink` / `LogSinkKind` / `LogFormat` / `Rotation` / `LineQueue` /
`Overflow`, `TracingConfig` / `ExportBatch`, `MetricsConfig` / `MetricsExporter` /
`PrometheusConfig` / `PrometheusHistograms`, and `OtlpConfig` / `OtlpProtocol` / `OtlpTls` /
`TlsRoots` / `ClientCert`. The runtime gate helpers (`metrics_enabled`,
`set_metrics_enabled`) and the whole `observability_core` crate are re-exported, so a binary
drives the hot-path gate through `observability` without a second dependency line.

Each config type implements `Default`. Together the defaults build a working pipeline that
doesn't need a tokio runtime: `ObsConfig::default()` logs JSON to stdout at `info` for service
`unknown_service` and leaves `otlp`, `tracing`, and `metrics` unset. Set the fields that differ
and end the literal with `..Default::default()`. The types have no `#[non_exhaustive]`, which
keeps struct literals working outside the crate; a minor release before 1.0 may still add
fields.

### Config files

With the `serde` feature, each config type also implements `Deserialize`. A service can then
read its whole config from TOML, YAML, or JSON:

```toml
[pipeline]
service_name = "feed-handler"

[pipeline.otlp]
endpoint = "https://collector.example:4318"
timeout_ms = 5000
headers = { authorization = "Bearer <token>" }
resource = { "deployment.environment" = "prod" }

[[pipeline.logging.sinks]]
kind = "stdout"

[[pipeline.logging.sinks]]
kind = { rolling_file = { dir = "/var/log/feed", prefix = "feed", max_files = 7 } }
queue = { lines = 50000, overflow = "block" }

[pipeline.tracing]
sample_ratio = 0.1
batch = { queue_size = 4096, delay_ms = 2000 }

[metrics.exporter.prometheus]
bind = "0.0.0.0:9000"
idle_timeout_ms = 300000
```

1. Names are snake_case, and an unknown field fails with its name, inside a `rolling_file` or
   `prometheus` table too.
2. An omitted field takes its type's `Default`. A partial table, such as `otlp_batch` with only
   `queue_size`, fills the rest from that table type's `Default`.
3. A `Duration` is integer milliseconds in a field ending in `_ms`: `timeout_ms`, `delay_ms`,
   `idle_timeout_ms`.
4. An enum is a string for a unit variant (`kind = "stdout"`) and a one-key table otherwise
   (`kind = { rolling_file = { ... } }`).
5. Headers and resource attributes are tables, one entry per key.
6. There is no `Serialize`, because it would write header values, usually credentials, back
   out.

Parse the file with the format crate of your choice, for example
`toml::from_str::<ObsConfig>(&text)?`, then pass the value to `init`.

## Tracing level and sampling

`TracingConfig::level` filters exported spans. Without it, the pipeline `level` applies, and a
set `RUST_LOG` overrides both, as it does for every sink.

`TracingConfig::sample_ratio` samples root spans with that probability; a child follows its
parent's decision, remote parents included. Init rejects a NaN ratio, or one outside
`0.0..=1.0`, with `PipelineError::SampleRatio`. With no ratio the SDK default sampler stays. It
reads `OTEL_TRACES_SAMPLER`, which keeps the other samplers available.

## Trace context propagation

With tracing on, `init` sets the W3C trace context propagator as the process-wide OTel text map
propagator, so OTel-aware HTTP and gRPC middleware works without setup. It replaces a
propagator set before `init`; a propagator set after `init` wins. `observability::propagation`
reads and writes `traceparent` and `tracestate` as plain string pairs, with no
`opentelemetry` dependency:

```rust,no_run
use observability::{
    PropagationError,
    propagation::{set_parent_from_headers, trace_headers},
};

fn handle(incoming: &[(&str, &str)]) -> Result<Vec<(String, String)>, PropagationError> {
    let span = tracing::info_span!("handle_request");
    // set the remote parent before the span is entered or its headers are read
    set_parent_from_headers(&span, incoming.iter().copied())?;
    let _entered = span.enter();

    // headers for an outgoing call made inside this span
    Ok(trace_headers(&span))
}
```

Header names match in any case. Without a valid `traceparent`, the span keeps its local parent
and the call returns `Ok`. Entering the span or reading its headers starts its OTel context and
fixes its parent, so a later `set_parent_from_headers` returns
`PropagationError::AlreadyStarted`. Baggage and other propagators are not covered.

## Log level precedence

Each sink's level filter is resolved as: a valid `RUST_LOG` value first, then the sink's own
`level` (or the pipeline `level` fallback), then `"info"`. Setting `RUST_LOG` therefore overrides
**every** sink's level uniformly, since each layer builds its filter from the same env var; the
per-sink `level` applies only while `RUST_LOG` is unset. An empty `RUST_LOG=""` counts as set and
yields `EnvFilter`'s empty default, not the config level.

## Rolling file sinks

`LogSinkKind::RollingFile { dir, prefix, rotation, max_files }` writes through a non-blocking
writer. A `dir` that cannot be created or opened returns `PipelineError::RollingFile` with the
directory. `max_files: Some(n)` keeps at most `n` files with the sink's prefix in `dir`. The
oldest are pruned when the sink opens and on each rotation, which means a restart enforces the
limit too. `max_files: None` keeps every file.

## Delivery limits

Every queue between the app and a sink has a fixed bound, and every item lost to a full queue or
a failed export is counted.

Each stdout and rolling-file sink buffers at most `LogSink::queue.lines` lines (a `LineQueue`,
128,000 by default). With `Overflow::Drop`, the default, a line that finds the buffer full is
dropped and counted. `Overflow::Block` makes the logging thread wait for space and loses
nothing; never use it on a latency-sensitive thread, since a slow disk or stdout then stalls
that thread.

From span end to the end of its export, a span takes one of `TracingConfig::batch.queue_size`
slots (an `ExportBatch`, 2048 by default). The span that finds them all taken is dropped. So is
each span in a batch whose export fails. `PipelineGuard::dropped()` counts every drop, and so
does the `observability.dropped` counter, labeled by `signal` and `reason`. The SDK's
`OTEL_BSP_*` env vars are not read: `ExportBatch::queue_size`, `max_batch_size`, and `delay`
replace them, because counting drops exactly needs the queue size the crate sets.

The OTLP log sink works the same way with `LoggingConfig::otlp_batch`, whose default exports a
partial batch after 1 s, and the SDK's `OTEL_BLRP_*` env vars are not read either. A config
may have at most one `LogSinkKind::Otlp` sink; a second one returns
`PipelineError::DuplicateOtlpLogSink`.

| SDK env var, not read | Set instead |
| --- | --- |
| `OTEL_BSP_MAX_QUEUE_SIZE`, `OTEL_BLRP_MAX_QUEUE_SIZE` | `ExportBatch::queue_size` |
| `OTEL_BSP_MAX_EXPORT_BATCH_SIZE`, `OTEL_BLRP_MAX_EXPORT_BATCH_SIZE` | `ExportBatch::max_batch_size` |
| `OTEL_BSP_SCHEDULE_DELAY`, `OTEL_BLRP_SCHEDULE_DELAY` | `ExportBatch::delay` |
| `OTEL_BSP_EXPORT_TIMEOUT`, `OTEL_BLRP_EXPORT_TIMEOUT` | `OtlpConfig::timeout` |

The counter's `reason` label is `queue_full`, `export_failed`, or `buffer_full`, and Prometheus
writes it as `observability_dropped`. A drop that happens before `init_metrics` installs a
recorder is counted only in the guard's totals, which a service without a recorder can read
directly:

```rust,no_run
fn report_drops(guard: &observability::PipelineGuard) {
    let lost = guard.dropped();
    tracing::info!(traces = lost.traces, logs = lost.logs, "telemetry dropped since init");
}
```

## Prometheus recorder

`MetricsExporter::Prometheus(PrometheusConfig)` serves a pull endpoint:

1. `bind` defaults to `127.0.0.1:9000`, so a default build does not serve metrics on every
   interface. A container scraped from outside sets `0.0.0.0:9000`.
2. `histograms` defaults to `PrometheusHistograms::Buckets` with the Prometheus client default
   seconds buckets, `[0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]`, which
   combine across instances in PromQL. `PrometheusHistograms::Summaries` exports quantile
   summaries instead.
3. `idle_timeout` (`idle_timeout_ms` in a file) drops a counter or histogram series not updated
   for that long, so changing label values do not grow memory until exit. Gauges are kept. A
   counter that comes back starts at zero, which `rate()` reads as a reset. The default is
   `None`.
4. Each series has a `service_name` label set to the service name passed to `init_metrics`.

The exporter builds with only its `http-listener` feature, without the push-gateway client and
its `aws-lc-rs` TLS provider.

## Building without backends

With `--no-default-features`, the otel and recorder wiring is compiled out. Config that needs a
disabled feature returns `PipelineError::FeatureDisabled` naming it: tracing without `otel`, an
OTLP log sink without `otel-logs`, a bunyan sink without `bunyan`, or an `https://` endpoint
without `tls` (`OtlpError::FeatureDisabled` inside `PipelineError::Exporter`). The
`--feature-powerset --depth 2` CI job builds the combinations.

## MSRV

Rust 1.88.0. The MSRV is the highest `rust-version` among the packages Cargo resolves for the CI
platforms, with all features and targets, and the CI MSRV cell builds and tests on it. The
workspace uses `resolver = "3"`, so `cargo update` keeps dependencies at versions that fit.

## License

MIT OR Apache-2.0.
