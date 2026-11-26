//! TLS settings for `https://` OTLP endpoints, and the checked material both transports
//! build their TLS config from.

use std::path::PathBuf;

#[cfg(feature = "tls")]
use opentelemetry_otlp::tonic_types::transport::ClientTlsConfig;

#[cfg(feature = "tls")]
use crate::error::OtlpError;

/// TLS settings for `https://` OTLP endpoints. `http://` endpoints ignore them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
pub enum TlsRoots {
    /// The OS certificate store.
    #[default]
    Native,
    /// Mozilla roots compiled into the binary, for images without an OS CA bundle. Needs
    /// the `tls-webpki-roots` feature.
    Webpki,
    /// Only the certificates in [`OtlpTls::ca_file`].
    CaFileOnly,
}

/// Client certificate and key for collectors that require mutual TLS.
#[derive(Clone, Debug, PartialEq, Eq)]
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
}

#[cfg(feature = "tls")]
impl TlsMaterial {
    pub(crate) fn load(tls: &OtlpTls) -> Result<Self, OtlpError> {
        match tls.roots {
            TlsRoots::Native => require_native_roots()?,
            TlsRoots::Webpki => require_webpki_feature()?,
            TlsRoots::CaFileOnly if tls.ca_file.is_none() => {
                return Err(OtlpError::NoTrustAnchors);
            }
            TlsRoots::CaFileOnly => {}
        }
        Ok(Self { roots: tls.roots })
    }

    pub(crate) fn tonic_config(&self) -> ClientTlsConfig {
        match self.roots {
            TlsRoots::Native => ClientTlsConfig::new().with_native_roots(),
            #[cfg(feature = "tls-webpki-roots")]
            TlsRoots::Webpki => ClientTlsConfig::new().with_webpki_roots(),
            _ => ClientTlsConfig::new(),
        }
    }

    pub(crate) fn apply_to_reqwest(
        &self,
        builder: reqwest::ClientBuilder,
    ) -> reqwest::ClientBuilder {
        // start from no roots, then add exactly the ones configured
        let builder = builder.use_rustls_tls().tls_built_in_root_certs(false);
        match self.roots {
            TlsRoots::Native => builder.tls_built_in_native_certs(true),
            #[cfg(feature = "tls-webpki-roots")]
            TlsRoots::Webpki => builder.tls_built_in_webpki_certs(true),
            _ => builder,
        }
    }
}

/// tonic fails its build on an empty OS store but reqwest only fails at the first
/// connect, so the check runs here and both transports fail the same way at init.
#[cfg(feature = "tls")]
fn require_native_roots() -> Result<(), OtlpError> {
    if rustls_native_certs::load_native_certs().certs.is_empty() {
        return Err(OtlpError::NoNativeRoots);
    }
    Ok(())
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
    use super::{OtlpTls, TlsMaterial, TlsRoots};
    use crate::error::OtlpError;

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
