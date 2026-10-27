//! Optional PEM trust anchors for LDAP and NetBox.
//!
//! An empty path keeps the system certificate store. A set path adds those
//! certificates and leaves verification on, even when the insecure flag is set.
//! A path that cannot be read fails the connection.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use rustls::pki_types::{pem::PemObject, CertificateDer};
use rustls::{ClientConfig, RootCertStore};

const MAX_CA_BYTES: u64 = 1_048_576;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TlsPosture {
    Default,
    Insecure,
    CustomCa,
    CaOverridesInsecure,
}

pub fn posture(insecure: bool, ca_file: &str) -> Result<TlsPosture, String> {
    let has_ca = absolute_ca_path(ca_file)?.is_some();
    Ok(match (has_ca, insecure) {
        (true, true) => TlsPosture::CaOverridesInsecure,
        (true, false) => TlsPosture::CustomCa,
        (false, true) => TlsPosture::Insecure,
        (false, false) => TlsPosture::Default,
    })
}

pub fn absolute_ca_path(raw: &str) -> Result<Option<PathBuf>, String> {
    let path = raw.trim();
    if path.is_empty() {
        return Ok(None);
    }
    if path.chars().count() > 400 || path.chars().any(char::is_control) {
        return Err("the_ca_file_must_be_an_absolute_path".into());
    }
    let path = Path::new(path);
    let escapes = path
        .components()
        .any(|component| matches!(component, Component::ParentDir));
    if !path.is_absolute() || escapes {
        return Err("the_ca_file_must_be_an_absolute_path".into());
    }
    Ok(Some(path.to_path_buf()))
}

pub fn read_ca_pem(path: &Path) -> Result<Vec<u8>, String> {
    let meta = std::fs::metadata(path).map_err(|_| "the_ca_file_cannot_be_read".to_string())?;
    if !meta.is_file() {
        return Err("the_ca_file_cannot_be_read".into());
    }
    if meta.len() > MAX_CA_BYTES {
        return Err("the_ca_file_is_too_large".into());
    }
    let bytes = std::fs::read(path).map_err(|_| "the_ca_file_cannot_be_read".to_string())?;
    if bytes.len() as u64 > MAX_CA_BYTES {
        return Err("the_ca_file_is_too_large".into());
    }
    Ok(bytes)
}

pub fn reqwest_roots(ca_file: &str) -> Result<Option<Vec<reqwest::Certificate>>, String> {
    let Some(path) = absolute_ca_path(ca_file)? else {
        return Ok(None);
    };
    let pem = read_ca_pem(&path)?;
    let certs = reqwest::Certificate::from_pem_bundle(&pem)
        .map_err(|_| "the_ca_file_is_not_a_certificate".to_string())?;
    if certs.is_empty() {
        return Err("the_ca_file_is_not_a_certificate".into());
    }
    Ok(Some(certs))
}

pub fn ldap_client_config(ca_file: &str) -> Result<Option<Arc<ClientConfig>>, String> {
    let Some(path) = absolute_ca_path(ca_file)? else {
        return Ok(None);
    };
    let pem = read_ca_pem(&path)?;
    let extra = extra_certificates(&pem)?;
    let mut roots = RootCertStore::empty();
    let native = rustls_native_certs::load_native_certs();
    if native.errors.is_empty() {
        for cert in native.certs {
            let _ = roots.add(cert);
        }
    }
    for cert in extra {
        roots
            .add(cert)
            .map_err(|_| "the_ca_file_is_not_a_certificate".to_string())?;
    }
    Ok(Some(client_config(roots)?))
}

pub fn warn_directory(cfg: &crate::config::LdapConfig) {
    if cfg.url.is_empty() {
        return;
    }
    match posture(cfg.tls_insecure, &cfg.ca_file) {
        Ok(TlsPosture::Insecure) => {
            tracing::warn!(url = %cfg.url, "LDAP certificate verification is disabled");
        }
        Ok(TlsPosture::CaOverridesInsecure) => {
            tracing::warn!(
                url = %cfg.url,
                ca_file = %cfg.ca_file,
                "LDAP certificate checks stay on because a CA file is set; TLS insecure is ignored"
            );
            warn_if_unreadable(&cfg.url, &cfg.ca_file);
        }
        Ok(TlsPosture::CustomCa) => warn_if_unreadable(&cfg.url, &cfg.ca_file),
        Ok(TlsPosture::Default) => {}
        Err(error) => {
            tracing::warn!(
                url = %cfg.url,
                %error,
                "LDAP CA file cannot be used; directory sign-in will fail"
            );
        }
    }
}

pub fn warn_netbox(endpoint: &str, insecure: bool, ca_file: &str) {
    match posture(insecure, ca_file) {
        Ok(TlsPosture::Insecure) => {
            tracing::warn!(%endpoint, "NetBox certificate verification is disabled");
        }
        Ok(TlsPosture::CaOverridesInsecure) => {
            tracing::warn!(
                %endpoint,
                %ca_file,
                "NetBox certificate checks stay on because a CA file is set; validate_certs false is ignored"
            );
        }
        _ => {}
    }
}

fn warn_if_unreadable(url: &str, ca_file: &str) {
    if let Err(error) = ldap_client_config(ca_file) {
        tracing::warn!(
            %url,
            %error,
            "LDAP CA file cannot be used; directory sign-in will fail"
        );
    }
}

fn extra_certificates(pem: &[u8]) -> Result<Vec<CertificateDer<'static>>, String> {
    let mut found = Vec::new();
    for item in CertificateDer::pem_slice_iter(pem) {
        let cert = item.map_err(|_| "the_ca_file_is_not_a_certificate".to_string())?;
        found.push(cert);
    }
    if found.is_empty() {
        return Err("the_ca_file_is_not_a_certificate".into());
    }
    Ok(found)
}

fn client_config(roots: RootCertStore) -> Result<Arc<ClientConfig>, String> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let builder = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| "the_certificate_store_could_not_be_built".to_string())?;
    Ok(Arc::new(
        builder.with_root_certificates(roots).with_no_client_auth(),
    ))
}

#[cfg(test)]
pub(crate) const TEST_CA_PEM: &str = "-----BEGIN CERTIFICATE-----\n\
MIIDGTCCAgGgAwIBAgIUH4TLQp2ba/1A2f3aZF9rwYxm0VcwDQYJKoZIhvcNAQEL\n\
BQAwHDEaMBgGA1UEAwwRS29yZW9ncmFmIFRlc3QgQ0EwHhcNMjYxMDI3MTkxNzA1\n\
WhcNMzYxMDI0MTkxNzA1WjAcMRowGAYDVQQDDBFLb3Jlb2dyYWYgVGVzdCBDQTCC\n\
ASIwDQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEBAPII8Fff3WTCtmC9xUQ8Hv48\n\
CLWb+MfU5IZT48W0gHnYjUOljQ8s24BRy4iXLWXAWANlaiNFSMhgKmH/Aiy5qSxv\n\
iBSAaGyewEcv9ynLKFjR5xcM5ooApLmiPj5YTJUMVlWJWZRvWL2F2NeQJkL1bs7i\n\
V2H+Fl3C7nBfoHYKGkUzOE9MBKgIRJ8PBUgLfrPbLTBn6YLVe8sJNQg3z3FY7Akp\n\
97djY2Qfch4+x/8s3Tu0UNe+jHfvGP5PJAs7mcxHFdM0J3o5eBf99ccx+d2a4Hd9\n\
zc5pcRUm2q/CqD6eslINHTtUE7UrJBmpAWVBrzB/qTcNz3b6WjHGK/ImOOSvX6cC\n\
AwEAAaNTMFEwHQYDVR0OBBYEFO71bSF77IbHscI1oWf9bKyiQBoSMB8GA1UdIwQY\n\
MBaAFO71bSF77IbHscI1oWf9bKyiQBoSMA8GA1UdEwEB/wQFMAMBAf8wDQYJKoZI\n\
hvcNAQELBQADggEBAH1vqcZpweR505vwLiCnw6U3edEanTFNTFFWqPznLgw8Ek/3\n\
h07NXOjeI2ydL/DcpImWFqj7dP+PDwbeucuRPVllSMNxz0Quo/3eN7o/dO/yQ+ux\n\
9JdK+UGPohW0GsRcAxqj02Gi+B/idFRHJpi20F5nfuO2beNFeQSOaIqREX5DX5PX\n\
DRILiG0QsTOWRFYGDX/f6kQd4DnFxFdg2qZjeiMIbcah1qtn2x//SDCpGQvr0B6b\n\
3rx5+d50ULiMAh7TPk64SAeBZ5pv68d1WZR/k2IJSD3CE8Saj8eies7xfUestGjH\n\
brXsgVyAio1sRQxSJcl/xHi6wQ5ACssBTz1ZpuE=\n\
-----END CERTIFICATE-----\n";

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-ca-{name}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn empty_path_keeps_the_system_store() {
        assert_eq!(posture(false, "  ").unwrap(), TlsPosture::Default);
        assert_eq!(posture(true, "").unwrap(), TlsPosture::Insecure);
        assert!(reqwest_roots("").unwrap().is_none());
        assert!(ldap_client_config("").unwrap().is_none());
    }

    #[test]
    fn a_ca_path_overrides_the_insecure_flag() {
        assert_eq!(
            posture(true, "/etc/ssl/certs/internal.pem").unwrap(),
            TlsPosture::CaOverridesInsecure
        );
        assert_eq!(
            posture(false, "/etc/ssl/certs/internal.pem").unwrap(),
            TlsPosture::CustomCa
        );
    }

    #[test]
    fn a_relative_or_parent_path_is_rejected() {
        assert_eq!(
            absolute_ca_path("certs/ca.pem").unwrap_err(),
            "the_ca_file_must_be_an_absolute_path"
        );
        assert_eq!(
            absolute_ca_path("/tmp/../etc/ca.pem").unwrap_err(),
            "the_ca_file_must_be_an_absolute_path"
        );
    }

    #[test]
    fn a_pem_file_is_added_to_both_clients() {
        let dir = scratch("pem");
        let path = dir.join("ca.pem");
        std::fs::write(&path, TEST_CA_PEM).unwrap();
        let text = path.display().to_string();
        let certs = reqwest_roots(&text).unwrap().unwrap();
        assert_eq!(certs.len(), 1);
        assert!(ldap_client_config(&text).unwrap().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_or_plain_file_fails_closed() {
        let missing = absolute_ca_path("/no/such/koreograf-ca.pem")
            .unwrap()
            .unwrap();
        assert_eq!(
            read_ca_pem(&missing).unwrap_err(),
            "the_ca_file_cannot_be_read"
        );
        let dir = scratch("text");
        let path = dir.join("note.txt");
        std::fs::write(&path, "not a certificate").unwrap();
        assert_eq!(
            reqwest_roots(&path.display().to_string()).unwrap_err(),
            "the_ca_file_is_not_a_certificate"
        );
        let huge = dir.join("huge.pem");
        std::fs::write(&huge, vec![b'A'; (MAX_CA_BYTES as usize) + 1]).unwrap();
        assert_eq!(read_ca_pem(&huge).unwrap_err(), "the_ca_file_is_too_large");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
