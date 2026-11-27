//! A Prometheus scrape shows bucketed histograms and the `service_name` label, and an idle
//! counter expires while a gauge stays.
#![cfg(feature = "metrics-prom")]

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    thread,
    time::Duration,
};

use observability::{MetricsConfig, MetricsExporter, PrometheusConfig, init_metrics};

const SERVICE: &str = "scrape-test";

// a port that was free a moment ago; the exporter binds it again
fn free_port() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    listener.local_addr().expect("the listener has an address")
}

fn scrape(addr: SocketAddr) -> String {
    let mut stream = TcpStream::connect(addr).expect("the exporter listens");
    stream
        .write_all(b"GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .expect("send the scrape request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read the scrape response");
    response
}

fn series_lines(scrape: &str) -> Vec<&str> {
    let (_, body) = scrape
        .split_once("\r\n\r\n")
        .expect("the response has a body");
    body.lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
}

// @lat: [[tests#Prometheus recorder#Scrape shows buckets service label and expiry]]
#[test]
fn scrape_shows_buckets_service_label_and_expiry() {
    let addr = free_port();
    let cfg = MetricsConfig {
        exporter: MetricsExporter::Prometheus(PrometheusConfig {
            bind: addr,
            idle_timeout: Some(Duration::from_millis(300)),
            ..Default::default()
        }),
    };
    let _guard = init_metrics(cfg, None, SERVICE).expect("the recorder installs");

    metrics::histogram!("req.seconds").record(0.2);
    metrics::counter!("hits").increment(1);
    metrics::gauge!("limit").set(1.0);
    let first = scrape(addr);
    // idle expiry reads the wall clock, so wait past the timeout
    thread::sleep(Duration::from_secs(1));
    let second = scrape(addr);

    let first_lines = series_lines(&first);
    assert!(
        first_lines
            .iter()
            .any(|line| line.starts_with("req_seconds_bucket") && line.contains("le=\"0.25\"")),
        "{first}"
    );
    let label = format!("service_name=\"{SERVICE}\"");
    assert!(
        first_lines.iter().all(|line| line.contains(&label)),
        "{first}"
    );
    assert!(
        first_lines.iter().any(|line| line.starts_with("hits")),
        "{first}"
    );
    let second_lines = series_lines(&second);
    assert!(
        !second_lines.iter().any(|line| line.starts_with("hits")),
        "{second}"
    );
    assert!(
        second_lines.iter().any(|line| line.starts_with("limit")),
        "{second}"
    );
}
