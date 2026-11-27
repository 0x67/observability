//! A throwaway PKI for TLS tests: a CA, a server
//! certificate for `localhost` and `127.0.0.1`, and a client certificate, generated with
//! rcgen at test time so no private key or expiring certificate is committed.
//!
//! Include it with `#[path = "support/test_pki.rs"] mod test_pki;`. The server configs it
//! builds are the `tls` argument of the mock collectors' `start`.

use std::{fs, path::PathBuf, sync::Arc};

use observability::{ClientCert, OtlpTls, TlsRoots};
use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair,
};
use rustls::{
    RootCertStore, ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    server::WebPkiClientVerifier,
};
use tonic::transport::{Certificate, Identity, ServerTlsConfig};

/// The generated certificates, with the client-side files in a temp dir removed on drop.
pub struct TestPki {
    dir: PathBuf,
    ca: CertificateDer<'static>,
    ca_pem: String,
    server_cert: CertificateDer<'static>,
    server_cert_pem: String,
    server_key: Vec<u8>,
    server_key_pem: String,
}

/// Generate a PKI whose client files live in a temp dir named after `test`.
pub fn generate(test: &str) -> TestPki {
    // tonic's TLS server takes rustls's process-wide provider; the crate uses aws-lc-rs as well
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let mut ca_params = CertificateParams::new(Vec::<String>::new()).expect("CA params");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "observability test CA");
    let ca = CertifiedIssuer::self_signed(ca_params, KeyPair::generate().expect("CA key"))
        .expect("CA certificate");

    let server_key = KeyPair::generate().expect("server key");
    let mut server_params =
        CertificateParams::new(vec!["localhost".to_owned(), "127.0.0.1".to_owned()])
            .expect("server params");
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let server_cert = server_params
        .signed_by(&server_key, &ca)
        .expect("server certificate");

    let client_key = KeyPair::generate().expect("client key");
    let mut client_params = CertificateParams::new(vec!["observability-test-client".to_owned()])
        .expect("client params");
    client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    let client_cert = client_params
        .signed_by(&client_key, &ca)
        .expect("client certificate");

    let dir = std::env::temp_dir().join(format!("obs-pki-{test}-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("create the PKI dir");
    let write = |name: &str, contents: &str| {
        fs::write(dir.join(name), contents).expect("write a PKI file");
    };
    write("ca.pem", &ca.pem());
    write("client.pem", &client_cert.pem());
    write("client.key", &client_key.serialize_pem());

    TestPki {
        ca: ca.der().clone(),
        ca_pem: ca.pem(),
        server_cert: server_cert.der().clone(),
        server_cert_pem: server_cert.pem(),
        server_key: server_key.serialize_der(),
        server_key_pem: server_key.serialize_pem(),
        dir,
    }
}

impl TestPki {
    /// TLS that trusts only the test CA and presents the test client certificate.
    pub fn otlp_tls(&self) -> OtlpTls {
        OtlpTls {
            roots: TlsRoots::CaFileOnly,
            ca_file: Some(self.dir.join("ca.pem")),
            client_cert: Some(ClientCert {
                cert_file: self.dir.join("client.pem"),
                key_file: self.dir.join("client.key"),
            }),
        }
    }

    /// Server TLS for the HTTP mock, requiring a client certificate from the test CA.
    pub fn http_server_tls(&self) -> Arc<ServerConfig> {
        let mut roots = RootCertStore::empty();
        roots.add(self.ca.clone()).expect("trust the test CA");
        let verifier = WebPkiClientVerifier::builder(Arc::new(roots))
            .build()
            .expect("client verifier");
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.server_key.clone()));
        let config = ServerConfig::builder()
            .with_client_cert_verifier(verifier)
            .with_single_cert(vec![self.server_cert.clone(), self.ca.clone()], key)
            .expect("server TLS config");
        Arc::new(config)
    }

    /// Server TLS for the gRPC mock, requiring a client certificate from the test CA.
    pub fn grpc_server_tls(&self) -> ServerTlsConfig {
        ServerTlsConfig::new()
            .identity(Identity::from_pem(
                &self.server_cert_pem,
                &self.server_key_pem,
            ))
            .client_ca_root(Certificate::from_pem(&self.ca_pem))
    }
}

impl Drop for TestPki {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}
