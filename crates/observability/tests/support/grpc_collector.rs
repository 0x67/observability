//! In-process OTLP/gRPC collector on `127.0.0.1:0` that records every export as text.
//!
//! Include it with `#[path = "support/grpc_collector.rs"] mod grpc_collector;`.

use std::{
    fmt::{Debug, Write},
    sync::{Arc, Mutex, PoisonError},
    thread,
};

use opentelemetry_proto::tonic::collector::{
    logs::v1::{
        ExportLogsServiceRequest, ExportLogsServiceResponse,
        logs_service_server::{LogsService, LogsServiceServer},
    },
    metrics::v1::{
        ExportMetricsServiceRequest, ExportMetricsServiceResponse,
        metrics_service_server::{MetricsService, MetricsServiceServer},
    },
    trace::v1::{
        ExportTraceServiceRequest, ExportTraceServiceResponse,
        trace_service_server::{TraceService, TraceServiceServer},
    },
};
use tonic::{
    Request, Response, Status,
    metadata::KeyAndValueRef,
    transport::{Server, ServerTlsConfig, server::TcpIncoming},
};

/// Handle to a running mock; it serves until the test process exits.
pub struct GrpcCollector {
    received: Arc<Mutex<String>>,
}

/// Start the mock on its own runtime in a std thread and return its endpoint and a handle.
/// With `tls`, it serves gRPC over TLS (`https://127.0.0.1:<port>`); without, plain gRPC
/// (`http://127.0.0.1:<port>`). The calling thread gets no runtime.
pub fn start(tls: Option<ServerTlsConfig>) -> (String, GrpcCollector) {
    let scheme = if tls.is_some() { "https" } else { "http" };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("start the mock collector runtime");
    let incoming = {
        let _entered = runtime.enter();
        TcpIncoming::bind("127.0.0.1:0".parse().expect("loopback address"))
            .expect("bind the mock collector")
    };
    let endpoint = format!(
        "{scheme}://{}",
        incoming.local_addr().expect("mock collector address")
    );
    let received = Arc::new(Mutex::new(String::new()));

    let recorder = Recorder(Arc::clone(&received));
    thread::spawn(move || {
        let mut server = Server::builder();
        if let Some(tls) = tls {
            server = server.tls_config(tls).expect("mock collector TLS config");
        }
        let server = server
            .add_service(TraceServiceServer::new(recorder.clone()))
            .add_service(LogsServiceServer::new(recorder.clone()))
            .add_service(MetricsServiceServer::new(recorder))
            .serve_with_incoming(incoming);
        runtime.block_on(server).expect("mock collector serves");
    });

    (endpoint, GrpcCollector { received })
}

impl GrpcCollector {
    /// Every export so far: its ASCII metadata as `key: value` lines, then the request in
    /// `Debug` form, which spells out span names, log bodies, and metric names.
    pub fn received_text(&self) -> String {
        self.received
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Clone)]
struct Recorder(Arc<Mutex<String>>);

impl Recorder {
    fn record<T: Debug>(&self, request: &Request<T>) {
        let mut text = String::new();
        for entry in request.metadata().iter() {
            if let KeyAndValueRef::Ascii(key, value) = entry {
                let value = value.to_str().unwrap_or("<not ascii>");
                let _ = writeln!(text, "{key}: {value}");
            }
        }
        let _ = writeln!(text, "{:?}", request.get_ref());

        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_str(&text);
    }
}

#[tonic::async_trait]
impl TraceService for Recorder {
    async fn export(
        &self,
        request: Request<ExportTraceServiceRequest>,
    ) -> Result<Response<ExportTraceServiceResponse>, Status> {
        self.record(&request);
        Ok(Response::new(ExportTraceServiceResponse::default()))
    }
}

#[tonic::async_trait]
impl LogsService for Recorder {
    async fn export(
        &self,
        request: Request<ExportLogsServiceRequest>,
    ) -> Result<Response<ExportLogsServiceResponse>, Status> {
        self.record(&request);
        Ok(Response::new(ExportLogsServiceResponse::default()))
    }
}

#[tonic::async_trait]
impl MetricsService for Recorder {
    async fn export(
        &self,
        request: Request<ExportMetricsServiceRequest>,
    ) -> Result<Response<ExportMetricsServiceResponse>, Status> {
        self.record(&request);
        Ok(Response::new(ExportMetricsServiceResponse::default()))
    }
}
