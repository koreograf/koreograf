//! Company certificate authorities from a fixed directory.
//!
//! Every `*.pem` and `*.crt` file in the directory is added to the public
//! roots for LDAP and NetBox. Git receives the same files through a bundle.
//! A missing or empty directory leaves trust unchanged.

use std::path::Path;
use std::sync::Arc;

use rustls::pki_types::{pem::PemObject, CertificateDer};
use rustls::{ClientConfig, RootCertStore};

pub(crate) const CA_DIRECTORY: &str = "/etc/koreograf/ca";
const MAX_CA_BYTES: u64 = 1_048_576;

#[derive(Debug, Default)]
pub(crate) struct CompanyCerts {
    pub certs: Vec<CertificateDer<'static>>,
    pub pem: Vec<u8>,
}

pub(crate) fn load_company_cas() -> Result<CompanyCerts, String> {
    load_cas(Path::new(CA_DIRECTORY))
}

pub(crate) fn load_cas(dir: &Path) -> Result<CompanyCerts, String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CompanyCerts::default())
        }
        Err(_) => {
            return Err(format!(
                "The CA directory {} cannot be read.",
                dir.display()
            ))
        }
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_ca_file(path))
        .collect::<Vec<_>>();
    paths.sort();
    let mut loaded = CompanyCerts::default();
    for path in paths {
        let pem = read_file(&path)?;
        let certs = parse_certificates(&pem, &path)?;
        loaded.certs.extend(certs);
        loaded.pem.extend(pem);
        if !loaded.pem.ends_with(b"\n") {
            loaded.pem.push(b'\n');
        }
    }
    Ok(loaded)
}

pub(crate) fn validate_company_cas() -> Result<(), String> {
    load_company_cas().map(|_| ())
}

pub(crate) fn reqwest_roots() -> Result<Vec<reqwest::Certificate>, String> {
    reqwest_roots_from(Path::new(CA_DIRECTORY))
}

fn reqwest_roots_from(dir: &Path) -> Result<Vec<reqwest::Certificate>, String> {
    let loaded = load_cas(dir)?;
    loaded
        .certs
        .into_iter()
        .map(|cert| {
            reqwest::Certificate::from_der(cert.as_ref())
                .map_err(|_| "A company CA certificate cannot be used.".to_string())
        })
        .collect()
}

pub(crate) fn ldap_client_config() -> Result<Option<Arc<ClientConfig>>, String> {
    ldap_client_config_from(Path::new(CA_DIRECTORY))
}

fn ldap_client_config_from(dir: &Path) -> Result<Option<Arc<ClientConfig>>, String> {
    let loaded = load_cas(dir)?;
    if loaded.certs.is_empty() {
        return Ok(None);
    }
    let mut roots = RootCertStore::empty();
    let native = rustls_native_certs::load_native_certs();
    if !native.errors.is_empty() {
        tracing::warn!(
            errors = ?native.errors,
            "Some native root certificates could not be read; the public roots that loaded are kept"
        );
    }
    for cert in native.certs {
        let _ = roots.add(cert);
    }
    for cert in loaded.certs {
        roots
            .add(cert)
            .map_err(|_| "A company CA certificate cannot be used.".to_string())?;
    }
    Ok(Some(client_config(roots)?))
}

pub(crate) fn warn_directory(cfg: &crate::config::LdapConfig) {
    if !cfg.url.is_empty() && cfg.tls_insecure {
        tracing::warn!(
            url = %cfg.url,
            "LDAP certificate verification is disabled"
        );
    }
}

pub(crate) fn warn_netbox(endpoint: &str, insecure: bool) {
    if insecure {
        tracing::warn!(%endpoint, "NetBox certificate verification is disabled");
    }
}

fn is_ca_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pem") || ext.eq_ignore_ascii_case("crt"))
}

fn read_file(path: &Path) -> Result<Vec<u8>, String> {
    let meta = std::fs::metadata(path)
        .map_err(|_| format!("The CA file {} cannot be read.", path.display()))?;
    if !meta.is_file() || meta.len() > MAX_CA_BYTES {
        return Err(format!("The CA file {} cannot be read.", path.display()));
    }
    let bytes = std::fs::read(path)
        .map_err(|_| format!("The CA file {} cannot be read.", path.display()))?;
    if bytes.len() as u64 > MAX_CA_BYTES {
        return Err(format!("The CA file {} cannot be read.", path.display()));
    }
    Ok(bytes)
}

fn parse_certificates(pem: &[u8], path: &Path) -> Result<Vec<CertificateDer<'static>>, String> {
    let mut found = Vec::new();
    for item in CertificateDer::pem_slice_iter(pem) {
        let cert =
            item.map_err(|_| format!("The CA file {} is not a PEM certificate.", path.display()))?;
        found.push(cert);
    }
    if found.is_empty() {
        return Err(format!(
            "The CA file {} is not a PEM certificate.",
            path.display()
        ));
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
    use std::path::PathBuf;

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
    fn a_missing_directory_keeps_the_public_roots() {
        let dir = scratch("missing").join("absent");
        assert!(load_cas(&dir).unwrap().certs.is_empty());
        assert!(reqwest_roots_from(&dir).unwrap().is_empty());
        assert!(ldap_client_config_from(&dir).unwrap().is_none());
    }

    #[test]
    fn an_empty_directory_keeps_the_public_roots() {
        let dir = scratch("empty");
        assert!(load_cas(&dir).unwrap().certs.is_empty());
        assert!(reqwest_roots_from(&dir).unwrap().is_empty());
        assert!(ldap_client_config_from(&dir).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_pem_and_crt_file_is_added_to_both_clients() {
        let dir = scratch("pem");
        std::fs::write(dir.join("company.pem"), TEST_CA_PEM).unwrap();
        std::fs::write(dir.join("company.crt"), TEST_CA_PEM).unwrap();
        std::fs::write(dir.join("notes.txt"), "not a certificate").unwrap();
        let loaded = load_cas(&dir).unwrap();
        assert_eq!(loaded.certs.len(), 2);
        assert_eq!(reqwest_roots_from(&dir).unwrap().len(), 2);
        assert!(ldap_client_config_from(&dir).unwrap().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_non_pem_file_fails_with_its_name() {
        let dir = scratch("text");
        std::fs::write(dir.join("broken.pem"), "not a certificate").unwrap();
        let error = load_cas(&dir).unwrap_err();
        assert!(error.contains("broken.pem"), "{error}");
        assert!(error.contains("is not a PEM certificate"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreadable_entry_fails_with_its_name() {
        let dir = scratch("unreadable");
        std::fs::create_dir(dir.join("broken.crt")).unwrap();
        let error = load_cas(&dir).unwrap_err();
        assert!(error.contains("broken.crt"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_huge_file_fails_with_its_name() {
        let dir = scratch("huge");
        std::fs::write(
            dir.join("huge.pem"),
            vec![b'A'; (MAX_CA_BYTES as usize) + 1],
        )
        .unwrap();
        let error = load_cas(&dir).unwrap_err();
        assert!(error.contains("huge.pem"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
