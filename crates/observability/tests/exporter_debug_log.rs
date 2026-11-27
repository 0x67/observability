//! Building a gRPC exporter logs the env header names at debug level, never their values.
#![cfg(feature = "metrics-otel")]

use std::{
    io,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use observability::{MetricsConfig, MetricsExporter, OtlpConfig, OtlpProtocol, init_metrics};
use tracing::Level;

/// In-memory sink for the fmt layer's output.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// @lat: [[tests#Exporter debug log#gRPC exporter build logs header keys without values]]
#[test]
fn grpc_exporter_build_logs_header_keys_without_values() {
    // SAFETY: the only test in this binary, and no other thread has started yet
    unsafe { std::env::set_var("OTEL_EXPORTER_OTLP_HEADERS", "x-api-key=sentinel-7c1f") };
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let otlp = OtlpConfig {
        endpoint: "http://127.0.0.1:4317".to_owned(),
        protocol: OtlpProtocol::Grpc,
        timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let cfg = MetricsConfig {
        exporter: MetricsExporter::Otlp,
    };

    let guard = tracing::subscriber::with_default(subscriber, || {
        init_metrics(cfg, Some(&otlp), "exporter-debug-log")
    })
    .expect("metrics install");
    drop(guard);

    let text = captured.text();
    assert!(text.contains("TonicChannelBuilt"), "{text}");
    assert!(text.contains("x-api-key"), "{text}");
    assert!(!text.contains("sentinel-7c1f"), "{text}");
}
