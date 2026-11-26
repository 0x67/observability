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

- `Prometheus { bind }`: a pull endpoint on the given socket.
- `Otlp`: an OTLP push bridge over the shared `OtlpConfig`.

The `observability-core` gate flips true only on a successful install; any error path returns
before touching it. An OTLP exporter that fails to build returns `MetricsError::Exporter`.

### `init`: both at once

```rust
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

### TLS

TLS applies only to `https://` endpoints; `http://localhost` stays plain text. `OtlpConfig::tls`
holds an `OtlpTls`:

```rust
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
| `metrics-prom` | yes | `metrics-exporter-prometheus` | Prometheus pull recorder |
| `metrics-otel` | no | `otel` + `metrics-exporter-otel` | OTLP metrics push bridge |
| `bunyan` | no | `tracing-bunyan-formatter` | bunyan-format log sink |

`default = ["logging", "otel", "tls", "metrics-prom"]`.

The whole otel stack pins to opentelemetry 0.31: `metrics-exporter-otel` 0.3.1 pins
`opentelemetry ^0.31`, and all OTLP signal exporters plus the metrics bridge must resolve to one
`opentelemetry` version, so that crate is the version bottleneck. Do not bump one otel crate
alone.

## Errors

```rust
pub enum PipelineError {
    AlreadyInit,
    MissingOtlp(Signal),                                  // tracing or an OTLP log sink without `otlp`
    FeatureDisabled { feature: &'static str },            // config needs a cargo feature the build lacks
    RollingFile { dir: PathBuf, source: Box<dyn Error + Send + Sync> },
    Exporter { signal: Signal, source: OtlpError },
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
    InvalidHeader { name: String },
    Build(Box<dyn Error + Send + Sync>),                  // the exporter or HTTP client did not build
}
pub enum MetricsError {
    Bind(io::Error),
    Install(Box<dyn Error + Send + Sync>),
    Exporter(OtlpError),
}
```

Upstream errors are boxed, so no `opentelemetry`, `reqwest`, `tonic`, or `tracing-appender`
type appears in a public signature; the install site picks the source.

## Config surface

`PipelineConfig` and its parts are plain data with no `opentelemetry` type in any field:
`LoggingConfig` / `LogSink` / `LogSinkKind` / `LogFormat` / `Rotation`, `TracingConfig` /
`TraceExporter`, `MetricsConfig` / `MetricsExporter`, and `OtlpConfig` / `OtlpProtocol` /
`OtlpTls` / `TlsRoots` / `ClientCert`. The
runtime gate helpers (`metrics_enabled`, `refresh_thread_gate`, `set_metrics_enabled`,
`METRICS_ON`) and the whole `observability_core` crate are re-exported, so a binary drives the
hot-path gate through `observability` without a second dependency line.

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

## Building without backends

With `--no-default-features`, the otel and recorder wiring is compiled out. Config that needs a
disabled feature returns `PipelineError::FeatureDisabled` naming it: tracing without `otel`, an
OTLP log sink without `otel-logs`, a bunyan sink without `bunyan`, or an `https://` endpoint
without `tls` (`OtlpError::FeatureDisabled` inside `PipelineError::Exporter`). The
`--feature-powerset --depth 2` CI job builds the combinations.

## License

MIT OR Apache-2.0.
