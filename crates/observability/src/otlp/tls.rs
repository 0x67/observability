//! TLS settings for `https://` OTLP endpoints, and the checked material both transports
//! build their TLS config from.

use std::path::PathBuf;
#[cfg(feature = "tls")]
use std::{path::Path, sync::Arc};

#[cfg(feature = "tls")]
use opentelemetry_otlp::tonic_types::transport::{Certificate, ClientTlsConfig, Identity};
#[cfg(feature = "tls")]
use rustls::{
    ClientConfig, RootCertStore,
    crypto::CryptoProvider,
    pki_types::{CertificateDer, PrivateKeyDer, pem, pem::PemObject},
};
#[cfg(feature = "tls-platform-verifier")]
use rustls_platform_verifier::Verifier;

#[cfg(feature = "tls")]
use crate::error::OtlpError;

/// TLS settings for `https://` OTLP endpoints. `http://` endpoints ignore them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default, deny_unknown_fields))]
pub struct OtlpTls {
    /// Root certificates to trust.
    pub roots: TlsRoots,
    /// PEM file with CA certificates to trust as well, for self-signed or internal-PKI
    /// collectors.
    pub ca_file: Option<PathBuf>,
    /// Client certificate and key, for collectors that require mutual TLS.
    pub client_cert: Option<ClientCert>,
}

/// Root certificates an `https://` collector's certificate is checked against.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum TlsRoots {
    /// The OS certificate store.
    #[default]
    Native,
    /// Mozilla roots compiled into the binary, for images without an OS CA bundle. Needs
    /// the `tls-webpki-roots` feature.
    Webpki,
    /// The OS verifier (Security.framework, the Windows certificate APIs, Android's trust
    /// manager, or webpki over the OS store elsewhere), which follows corporate CAs and
    /// revocation; [`OtlpTls::ca_file`] adds extra roots except on Android. For desktop and
    /// mobile apps. Needs the `tls-platform-verifier` feature, and an Android app must call
    /// `rustls_platform_verifier::android::init_with_env` first.
    Platform,
    /// Only the certificates in [`OtlpTls::ca_file`].
    CaFileOnly,
}

/// Client certificate and key for collectors that require mutual TLS.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct ClientCert {
    /// PEM certificate chain, leaf first.
    pub cert_file: PathBuf,
    /// PEM private key (PKCS#8, PKCS#1, or SEC1).
    pub key_file: PathBuf,
}

/// TLS inputs read and checked at init, ready for either transport.
#[cfg(feature = "tls")]
pub(crate) struct TlsMaterial {
    roots: TlsRoots,
    // the OS store for `TlsRoots::Native`, empty for the other roots
    native_roots: Vec<CertificateDer<'static>>,
    ca_pem: Option<Vec<u8>>,
    client: Option<ClientPem>,
}

/// A client certificate chain and its key, as read from disk.
#[cfg(feature = "tls")]
struct ClientPem {
    chain: Vec<u8>,
    key: Vec<u8>,
}

#[cfg(feature = "tls")]
impl TlsMaterial {
    pub(crate) fn load(tls: &OtlpTls) -> Result<Self, OtlpError> {
        let native_roots = match tls.roots {
            TlsRoots::Native => native_roots()?,
            TlsRoots::Webpki => require_webpki_feature().map(|()| Vec::new())?,
            TlsRoots::Platform => require_platform_feature().map(|()| Vec::new())?,
            TlsRoots::CaFileOnly if tls.ca_file.is_none() => {
                return Err(OtlpError::NoTrustAnchors);
            }
            TlsRoots::CaFileOnly => Vec::new(),
        };

        let ca_pem = tls.ca_file.as_deref().map(read_certificates).transpose()?;
        let client = tls.client_cert.as_ref().map(read_client_pem).transpose()?;
        Ok(Self {
            roots: tls.roots,
            native_roots,
            ca_pem,
            client,
        })
    }

    pub(crate) fn tonic_config(&self) -> ClientTlsConfig {
        let config = match self.roots {
            TlsRoots::Native => ClientTlsConfig::new().with_native_roots(),
            #[cfg(feature = "tls-webpki-roots")]
            TlsRoots::Webpki => ClientTlsConfig::new().with_webpki_roots(),
            _ => ClientTlsConfig::new(),
        };
        let config = match &self.ca_pem {
            Some(pem) => config.ca_certificate(Certificate::from_pem(pem)),
            None => config,
        };
        match &self.client {
            Some(client) => config.identity(Identity::from_pem(&client.chain, &client.key)),
            None => config,
        }
    }

    // reqwest 0.13 has no root store features, and its own config fails the build on any
    // OS certificate rustls cannot parse, so the crate hands it a finished rustls config
    pub(crate) fn apply_to_reqwest(
        &self,
        builder: reqwest::ClientBuilder,
    ) -> Result<reqwest::ClientBuilder, OtlpError> {
        Ok(builder.tls_backend_preconfigured(self.rustls_config()?))
    }

    /// rustls config for the HTTP client: the configured roots or the OS verifier, and the
    /// client certificate.
    fn rustls_config(&self) -> Result<ClientConfig, OtlpError> {
        let provider = process_or_aws_lc_provider();
        let builder = ClientConfig::builder_with_provider(Arc::clone(&provider))
            .with_safe_default_protocol_versions()
            .map_err(OtlpError::build)?;

        #[cfg(feature = "tls-platform-verifier")]
        let builder = if self.uses_platform_verifier() {
            // rustls names every custom verifier hook `dangerous`; this one checks the full chain
            builder
                .dangerous()
                .with_custom_certificate_verifier(self.platform_verifier(provider)?)
        } else {
            builder.with_root_certificates(self.root_store()?)
        };
        #[cfg(not(feature = "tls-platform-verifier"))]
        let builder = builder.with_root_certificates(self.root_store()?);

        // both PEM inputs parsed in `load`, so these errors are not expected here
        match &self.client {
            Some(client) => {
                let chain = pem_certificates(&client.chain).map_err(OtlpError::build)?;
                let key = PrivateKeyDer::from_pem_slice(&client.key).map_err(OtlpError::build)?;
                builder
                    .with_client_auth_cert(chain, key)
                    .map_err(OtlpError::build)
            }
            None => Ok(builder.with_no_client_auth()),
        }
    }

    fn root_store(&self) -> Result<RootCertStore, OtlpError> {
        let mut store = RootCertStore::empty();
        // an OS store can hold certificates rustls cannot parse; tonic skips them as well
        store.add_parsable_certificates(self.native_roots.iter().cloned());
        #[cfg(feature = "tls-webpki-roots")]
        if self.roots == TlsRoots::Webpki {
            store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        }
        if let Some(pem) = &self.ca_pem {
            for cert in pem_certificates(pem).map_err(OtlpError::build)? {
                store.add(cert).map_err(OtlpError::build)?;
            }
        }
        Ok(store)
    }
}

/// Read a PEM file and check it holds at least one certificate, so a bad CA or chain
/// fails at init with its path on either transport.
#[cfg(feature = "tls")]
fn read_certificates(path: &Path) -> Result<Vec<u8>, OtlpError> {
    let pem = read_tls_file(path)?;
    let invalid = |source| OtlpError::InvalidPem {
        path: path.to_path_buf(),
        source,
    };

    match pem_certificates(&pem) {
        Ok(certs) if !certs.is_empty() => Ok(pem),
        Ok(_) => Err(invalid("no PEM certificate found".into())),
        Err(e) => Err(invalid(Box::new(e))),
    }
}

/// Read the chain and the key, and check that both parse.
#[cfg(feature = "tls")]
fn read_client_pem(cert: &ClientCert) -> Result<ClientPem, OtlpError> {
    let client = ClientPem {
        chain: read_certificates(&cert.cert_file)?,
        key: read_tls_file(&cert.key_file)?,
    };

    PrivateKeyDer::from_pem_slice(&client.key).map_err(|e| OtlpError::InvalidPem {
        path: cert.key_file.clone(),
        source: Box::new(e),
    })?;
    Ok(client)
}

#[cfg(feature = "tls")]
fn read_tls_file(path: &Path) -> Result<Vec<u8>, OtlpError> {
    std::fs::read(path).map_err(|source| OtlpError::TlsFile {
        path: path.to_path_buf(),
        source,
    })
}

/// The OS store. tonic fails its build on an empty store, so an empty store is an error
/// here and both transports fail the same way at init.
#[cfg(feature = "tls")]
fn native_roots() -> Result<Vec<CertificateDer<'static>>, OtlpError> {
    let roots = rustls_native_certs::load_native_certs().certs;
    if roots.is_empty() {
        return Err(OtlpError::NoNativeRoots);
    }
    Ok(roots)
}

/// The process-wide rustls provider when the app installed one, aws-lc-rs otherwise, the
/// same choice tonic and reqwest make.
#[cfg(feature = "tls")]
fn process_or_aws_lc_provider() -> Arc<CryptoProvider> {
    CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
}

#[cfg(feature = "tls")]
fn pem_certificates(pem: &[u8]) -> Result<Vec<CertificateDer<'static>>, pem::Error> {
    CertificateDer::pem_slice_iter(pem).collect()
}

#[cfg(feature = "tls-platform-verifier")]
impl TlsMaterial {
    pub(crate) fn uses_platform_verifier(&self) -> bool {
        self.roots == TlsRoots::Platform
    }

    /// The pieces for tonic's `Endpoint::tls_config_with_verifier`: a config that carries
    /// only the client identity (the endpoint rejects any root or CA setting) and the OS
    /// verifier.
    pub(crate) fn platform_tonic(&self) -> Result<(ClientTlsConfig, Arc<Verifier>), OtlpError> {
        let config = match &self.client {
            Some(client) => {
                ClientTlsConfig::new().identity(Identity::from_pem(&client.chain, &client.key))
            }
            None => ClientTlsConfig::new(),
        };
        Ok((
            config,
            self.platform_verifier(process_or_aws_lc_provider())?,
        ))
    }

    /// The OS verifier, with `ca_file` certificates as extra roots where the platform
    /// allows it.
    fn platform_verifier(&self, provider: Arc<CryptoProvider>) -> Result<Arc<Verifier>, OtlpError> {
        let verifier = match &self.ca_pem {
            None => Verifier::new(provider),
            #[cfg(not(target_os = "android"))]
            Some(pem) => Verifier::new_with_extra_roots(
                pem_certificates(pem).map_err(platform_error)?,
                provider,
            ),
            #[cfg(target_os = "android")]
            Some(_) => {
                return Err(OtlpError::PlatformVerifier(
                    "ca_file is not supported with TlsRoots::Platform on Android".into(),
                ));
            }
        };
        verifier.map(Arc::new).map_err(platform_error)
    }
}

#[cfg(feature = "tls-platform-verifier")]
fn platform_error(source: impl std::error::Error + Send + Sync + 'static) -> OtlpError {
    OtlpError::PlatformVerifier(Box::new(source))
}

#[cfg(feature = "tls")]
fn require_platform_feature() -> Result<(), OtlpError> {
    if cfg!(feature = "tls-platform-verifier") {
        Ok(())
    } else {
        Err(OtlpError::FeatureDisabled {
            feature: "tls-platform-verifier",
        })
    }
}

#[cfg(feature = "tls")]
fn require_webpki_feature() -> Result<(), OtlpError> {
    if cfg!(feature = "tls-webpki-roots") {
        Ok(())
    } else {
        Err(OtlpError::FeatureDisabled {
            feature: "tls-webpki-roots",
        })
    }
}

#[cfg(all(test, feature = "tls"))]
mod tests {
    use std::{fs, path::PathBuf};

    use super::{ClientCert, OtlpTls, TlsMaterial, TlsRoots};
    use crate::error::OtlpError;

    /// A temp dir for the PEM files one test writes, removed on drop.
    struct PemDir(PathBuf);

    impl PemDir {
        fn new(test: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("obs-tls-{test}-{}", std::process::id()));
            fs::create_dir_all(&dir).expect("create the PEM dir");
            Self(dir)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }

        fn write(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.path(name);
            fs::write(&path, contents).expect("write a PEM file");
            path
        }
    }

    impl Drop for PemDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // a self-signed `localhost` certificate and its key, both PEM
    fn self_signed() -> (String, String) {
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
            .expect("generate a certificate");
        (certified.cert.pem(), certified.signing_key.serialize_pem())
    }

    fn with_files(ca_file: PathBuf, client: Option<(PathBuf, PathBuf)>) -> OtlpTls {
        OtlpTls {
            roots: TlsRoots::CaFileOnly,
            ca_file: Some(ca_file),
            client_cert: client.map(|(cert_file, key_file)| ClientCert {
                cert_file,
                key_file,
            }),
        }
    }

    #[test]
    fn unreadable_tls_file_returns_tls_file_with_path() {
        let dir = PemDir::new("unreadable");
        let (cert, key) = self_signed();
        let cert_file = dir.write("cert.pem", &cert);
        let key_file = dir.write("key.pem", &key);
        let missing = dir.path("missing.pem");

        for tls in [
            with_files(missing.clone(), None),
            with_files(cert_file.clone(), Some((missing.clone(), key_file.clone()))),
            with_files(
                cert_file.clone(),
                Some((cert_file.clone(), missing.clone())),
            ),
        ] {
            let loaded = TlsMaterial::load(&tls);

            assert!(
                matches!(&loaded, Err(OtlpError::TlsFile { path, .. }) if *path == missing),
                "{tls:?}: {:?}",
                loaded.err()
            );
        }
    }

    #[test]
    fn invalid_pem_returns_invalid_pem_with_path() {
        let dir = PemDir::new("invalid");
        let (cert, key) = self_signed();
        let cert_file = dir.write("cert.pem", &cert);
        let key_file = dir.write("key.pem", &key);
        let bad = dir.write("bad.pem", "not a pem");

        for tls in [
            with_files(bad.clone(), None),
            with_files(cert_file.clone(), Some((bad.clone(), key_file.clone()))),
            with_files(cert_file.clone(), Some((cert_file.clone(), bad.clone()))),
        ] {
            let loaded = TlsMaterial::load(&tls);

            assert!(
                matches!(&loaded, Err(OtlpError::InvalidPem { path, .. }) if *path == bad),
                "{tls:?}: {:?}",
                loaded.err()
            );
        }
    }

    #[cfg(not(feature = "tls-webpki-roots"))]
    #[test]
    fn webpki_roots_without_feature_returns_feature_disabled() {
        let tls = OtlpTls {
            roots: TlsRoots::Webpki,
            ..OtlpTls::default()
        };

        let loaded = TlsMaterial::load(&tls);

        assert!(matches!(
            loaded,
            Err(OtlpError::FeatureDisabled {
                feature: "tls-webpki-roots"
            })
        ));
    }

    #[cfg(not(feature = "tls-platform-verifier"))]
    #[test]
    fn platform_roots_without_feature_returns_feature_disabled() {
        let tls = OtlpTls {
            roots: TlsRoots::Platform,
            ..OtlpTls::default()
        };

        let loaded = TlsMaterial::load(&tls);

        assert!(matches!(
            loaded,
            Err(OtlpError::FeatureDisabled {
                feature: "tls-platform-verifier"
            })
        ));
    }

    #[test]
    fn ca_file_only_without_ca_file_returns_no_trust_anchors() {
        let tls = OtlpTls {
            roots: TlsRoots::CaFileOnly,
            ..OtlpTls::default()
        };

        let loaded = TlsMaterial::load(&tls);

        assert!(matches!(loaded, Err(OtlpError::NoTrustAnchors)));
    }
}
