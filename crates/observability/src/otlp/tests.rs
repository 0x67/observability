use std::time::Duration;

use super::{OtlpConfig, OtlpProtocol};

#[cfg(feature = "otel")]
fn exporter_config(protocol: OtlpProtocol, endpoint: &str) -> OtlpConfig {
    OtlpConfig {
        endpoint: endpoint.to_owned(),
        protocol,
        timeout: Duration::from_secs(1),
        ..Default::default()
    }
}

#[cfg(all(feature = "otel", not(feature = "tls")))]
#[test]
fn https_without_tls_feature_returns_feature_disabled() {
    use std::sync::PoisonError;

    use super::{ExportRuntime, runtime::tests::SLOT_LOCK};
    use crate::error::OtlpError;

    let _lock = SLOT_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let runtime = ExportRuntime::shared().expect("export runtime starts");

    for (protocol, endpoint) in [
        (OtlpProtocol::Grpc, "https://collector:4317"),
        (OtlpProtocol::HttpProtobuf, "https://collector:4318"),
        (OtlpProtocol::Grpc, "HTTPS://collector:4317"),
    ] {
        let built = exporter_config(protocol, endpoint).span_exporter(&runtime);

        assert!(
            matches!(built, Err(OtlpError::FeatureDisabled { feature: "tls" })),
            "{protocol:?} {endpoint}: {:?}",
            built.err()
        );
    }
}

// @lat: [[tests#OTLP signal paths#Signal URL appends the signal path]]
#[cfg(feature = "otel")]
#[test]
fn signal_url_appends_signal_path() {
    for (endpoint, path, want) in [
        ("http://c:4318", "/v1/traces", "http://c:4318/v1/traces"),
        ("http://c:4318/", "/v1/logs", "http://c:4318/v1/logs"),
        ("https://c/otlp", "/v1/metrics", "https://c/otlp/v1/metrics"),
    ] {
        let cfg = exporter_config(OtlpProtocol::HttpProtobuf, endpoint);

        assert_eq!(cfg.signal_url(path), want, "{endpoint} + {path}");
    }
}

// @lat: [[tests#OTLP headers#Invalid header is rejected on both protocols]]
#[cfg(feature = "otel")]
#[test]
fn invalid_header_is_rejected_on_both_protocols() {
    use std::sync::PoisonError;

    use super::{ExportRuntime, runtime::tests::SLOT_LOCK};
    use crate::error::OtlpError;

    let _lock = SLOT_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let runtime = ExportRuntime::shared().expect("export runtime starts");

    for (name, value) in [("x api", "k1"), ("x-api-key", "k1\n")] {
        for protocol in [OtlpProtocol::Grpc, OtlpProtocol::HttpProtobuf] {
            let cfg = OtlpConfig {
                headers: [(name.to_owned(), value.to_owned())].into(),
                ..exporter_config(protocol, "http://127.0.0.1:4317")
            };

            let err = cfg.span_exporter(&runtime).err();

            let Some(err @ OtlpError::InvalidHeader { name: named }) = &err else {
                panic!("{protocol:?} {name:?}: {err:?}");
            };
            assert_eq!(named, name);
            let text = err.to_string();
            assert!(text.contains(name) && !text.contains("k1"), "{text}");
        }
    }
}

// @lat: [[tests#OTLP endpoint credentials#Endpoint credentials are rejected before other checks]]
#[cfg(feature = "otel")]
#[test]
fn endpoint_credentials_are_rejected_before_other_checks() {
    use std::sync::PoisonError;

    use super::{ExportRuntime, runtime::tests::SLOT_LOCK};
    use crate::error::OtlpError;

    let _lock = SLOT_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let runtime = ExportRuntime::shared().expect("export runtime starts");

    for (protocol, endpoint) in [
        (
            OtlpProtocol::HttpProtobuf,
            "https://user:pa55word@collector:4318",
        ),
        (OtlpProtocol::Grpc, "http://user:pa55word@collector:4317"),
        (OtlpProtocol::Grpc, "http://token@collector:4317"),
        (OtlpProtocol::HttpProtobuf, "user:pa55word@collector:4318"),
    ] {
        // the header check comes next and would fail on this header
        let cfg = OtlpConfig {
            headers: [("x api".to_owned(), "v".to_owned())].into(),
            ..exporter_config(protocol, endpoint)
        };

        let err = cfg.span_exporter(&runtime).err();

        let Some(err @ OtlpError::EndpointCredentials) = &err else {
            panic!("{protocol:?} {endpoint}: {err:?}");
        };
        let text = format!("{err} {err:?}");
        assert!(
            !text.contains("pa55word") && !text.contains("collector"),
            "{text}"
        );
    }
}

// @lat: [[tests#OTLP endpoint credentials#At sign outside authority is not a credential]]
#[cfg(feature = "otel")]
#[test]
fn at_sign_outside_authority_is_not_a_credential() {
    use super::endpoint_has_credentials;

    for endpoint in [
        "http://collector:4318/v1/traces?x=a@b",
        "http://collector:4318/users/a@b/traces",
        "collector:4317",
    ] {
        assert!(!endpoint_has_credentials(endpoint), "{endpoint}");
    }
}

// @lat: [[tests#OTLP endpoint scheme#Endpoint without scheme is rejected on both protocols]]
#[cfg(feature = "otel")]
#[test]
fn endpoint_without_scheme_is_rejected_on_both_protocols() {
    use std::sync::PoisonError;

    use super::{ExportRuntime, runtime::tests::SLOT_LOCK};
    use crate::error::OtlpError;

    let _lock = SLOT_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let runtime = ExportRuntime::shared().expect("export runtime starts");

    for (protocol, endpoint) in [
        (OtlpProtocol::Grpc, "collector.example:4317"),
        (OtlpProtocol::HttpProtobuf, "collector.example:4318"),
        (OtlpProtocol::Grpc, "unix:///tmp/otel.sock"),
    ] {
        let err = exporter_config(protocol, endpoint)
            .span_exporter(&runtime)
            .err();

        let Some(OtlpError::EndpointScheme { endpoint: named }) = &err else {
            panic!("{protocol:?} {endpoint}: {err:?}");
        };
        assert_eq!(named, endpoint);
    }

    // the scheme is compared without case
    let cfg = exporter_config(OtlpProtocol::HttpProtobuf, "HTTP://127.0.0.1:4318");
    if let Err(e) = cfg.span_exporter(&runtime) {
        panic!("HTTP:// endpoint: {e}");
    }
}

// @lat: [[tests#OTLP endpoint scheme#Exporter follows configured protocol over env]]
#[cfg(feature = "otel")]
#[test]
fn exporter_follows_configured_protocol_over_env() {
    use std::sync::PoisonError;

    use super::{ExportRuntime, runtime::tests::SLOT_LOCK};

    let _lock = SLOT_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let runtime = ExportRuntime::shared().expect("export runtime starts");

    for (var, value, protocol, endpoint) in [
        (
            "OTEL_EXPORTER_OTLP_PROTOCOL",
            "grpc",
            OtlpProtocol::HttpProtobuf,
            "http://127.0.0.1:4318",
        ),
        (
            "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL",
            "grpc",
            OtlpProtocol::HttpProtobuf,
            "http://127.0.0.1:4318",
        ),
        (
            "OTEL_EXPORTER_OTLP_PROTOCOL",
            "http/protobuf",
            OtlpProtocol::Grpc,
            "http://127.0.0.1:4317",
        ),
    ] {
        let previous = std::env::var_os(var);
        // SAFETY: SLOT_LOCK serializes this with every exporter build in the otlp tests, and
        // nextest runs each test in its own process, so no other thread reads the environment.
        unsafe { std::env::set_var(var, value) };

        let built = exporter_config(protocol, endpoint).span_exporter(&runtime);

        // SAFETY: SLOT_LOCK is still held; put back what the test process started with.
        unsafe {
            match previous {
                Some(previous) => std::env::set_var(var, previous),
                None => std::env::remove_var(var),
            }
        }
        if let Err(e) = built {
            panic!("{var}={value} with {protocol:?}: {e}");
        }
    }
}

// needs an OS certificate store with at least one root, as on every CI image
#[cfg(feature = "tls")]
#[test]
fn https_endpoints_build_with_tls() {
    use std::sync::PoisonError;

    use super::{ExportRuntime, runtime::tests::SLOT_LOCK};

    let _lock = SLOT_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let runtime = ExportRuntime::shared().expect("export runtime starts");

    for (protocol, endpoint) in [
        (OtlpProtocol::Grpc, "https://collector:4317"),
        (OtlpProtocol::HttpProtobuf, "https://collector:4318"),
    ] {
        if let Err(e) = exporter_config(protocol, endpoint).span_exporter(&runtime) {
            panic!("{protocol:?} {endpoint}: {e}");
        }
    }
}

#[cfg(feature = "tls-platform-verifier")]
#[test]
fn platform_roots_build_both_transports() {
    use std::sync::PoisonError;

    use super::{ExportRuntime, OtlpTls, TlsRoots, runtime::tests::SLOT_LOCK};

    let _lock = SLOT_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let runtime = ExportRuntime::shared().expect("export runtime starts");

    for (protocol, endpoint) in [
        (OtlpProtocol::Grpc, "https://collector:4317"),
        (OtlpProtocol::HttpProtobuf, "https://collector:4318"),
    ] {
        let cfg = OtlpConfig {
            tls: OtlpTls {
                roots: TlsRoots::Platform,
                ..OtlpTls::default()
            },
            ..exporter_config(protocol, endpoint)
        };

        if let Err(e) = cfg.span_exporter(&runtime) {
            panic!("{protocol:?} {endpoint}: {e}");
        }
    }
}

#[cfg(feature = "otel")]
#[test]
fn exporters_build_without_app_runtime() {
    use std::sync::PoisonError;

    use super::{ExportRuntime, runtime::tests::SLOT_LOCK};

    let _lock = SLOT_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    assert!(tokio::runtime::Handle::try_current().is_err());
    let runtime = ExportRuntime::shared().expect("export runtime starts");

    for (protocol, endpoint) in [
        (OtlpProtocol::Grpc, "http://127.0.0.1:4317"),
        (OtlpProtocol::HttpProtobuf, "http://127.0.0.1:4318"),
    ] {
        let cfg = OtlpConfig {
            endpoint: endpoint.to_string(),
            protocol,
            timeout: Duration::from_secs(1),
            ..Default::default()
        };

        if let Err(e) = cfg.span_exporter(&runtime) {
            panic!("{protocol:?} span exporter: {e}");
        }
        #[cfg(feature = "otel-logs")]
        if let Err(e) = cfg.log_exporter(&runtime) {
            panic!("{protocol:?} log exporter: {e}");
        }
        #[cfg(feature = "metrics-otel")]
        if let Err(e) = cfg.metric_exporter(&runtime) {
            panic!("{protocol:?} metric exporter: {e}");
        }
    }
}

// @lat: [[tests#OTLP config debug output#Debug prints header names and hides values]]
#[test]
fn debug_prints_header_names_and_hides_values() {
    let cfg = OtlpConfig {
        endpoint: "http://collector:4318".to_string(),
        protocol: OtlpProtocol::HttpProtobuf,
        headers: [("x-api-key".to_string(), "s3cr3t-value".to_string())].into(),
        resource: [("deployment.environment".to_string(), "prod".to_string())].into(),
        timeout: Duration::from_secs(5),
        ..Default::default()
    };

    let out = format!("{cfg:?}");

    assert!(out.contains("\"x-api-key\", \"[REDACTED]\""), "{out}");
    assert!(!out.contains("s3cr3t-value"), "{out}");
    for shown in [
        "http://collector:4318",
        "HttpProtobuf",
        "deployment.environment",
        "prod",
        "5s",
    ] {
        assert!(out.contains(shown), "missing {shown}: {out}");
    }
}
