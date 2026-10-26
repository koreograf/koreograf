use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use pbkdf2::pbkdf2_hmac;
use sha2::Sha256;

use crate::error::{ApiError, AppResult};

const PBKDF2_ITERS: u32 = 120_000;

pub struct Cipher {
    key: [u8; 32],
}

impl Cipher {
    pub fn encrypt(&self, plain: &str) -> AppResult<String> {
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|_| ApiError::Internal("key is invalid".into()))?;
        let mut nonce_bytes = [0u8; 12];
        rand::fill(&mut nonce_bytes);
        let nonce = Nonce::from(nonce_bytes);
        let ct = cipher
            .encrypt(&nonce, plain.as_bytes())
            .map_err(|_| ApiError::Internal("encryption failed".into()))?;
        let mut packed = nonce_bytes.to_vec();
        packed.extend(ct);
        Ok(B64.encode(packed))
    }

    pub fn decrypt(&self, packed: &str) -> AppResult<String> {
        let bytes = B64
            .decode(packed.trim())
            .map_err(|_| ApiError::Internal("secret is not a valid record".into()))?;
        if bytes.len() < 13 {
            return Err(ApiError::Internal("secret is too short".into()));
        }
        let (nonce_bytes, ct) = bytes.split_at(12);
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|_| ApiError::Internal("key is invalid".into()))?;
        let nonce = Nonce::try_from(nonce_bytes)
            .map_err(|_| ApiError::Internal("secret cannot be read".into()))?;
        let plain = cipher
            .decrypt(&nonce, ct)
            .map_err(|_| ApiError::Internal("secret cannot be read".into()))?;
        String::from_utf8(plain).map_err(|_| ApiError::Internal("secret is not text".into()))
    }
}

pub(crate) fn parse_key(raw: &[u8]) -> AppResult<[u8; 32]> {
    decode_key(raw).map_err(|_| ApiError::bad("the_key_in_the_backup_is_invalid"))
}

pub(crate) fn install_key(data_dir: &Path, raw: &[u8]) -> AppResult<Cipher> {
    let key = parse_key(raw)?;
    write_key(&data_dir.join("secret.key"), &key)?;
    Ok(Cipher { key })
}

pub fn load_cipher(data_dir: &Path) -> AppResult<Cipher> {
    fs::create_dir_all(data_dir).map_err(|e| ApiError::Internal(e.to_string()))?;
    let path = data_dir.join("secret.key");
    if let Ok(raw) = fs::read(&path) {
        if let Ok(key) = decode_key(&raw) {
            return Ok(Cipher { key });
        }
    }
    if let Ok(env) = std::env::var("KOREOGRAF_SECRET_KEY") {
        if let Ok(key) = decode_key(env.trim().as_bytes()) {
            write_key(&path, &key)?;
            return Ok(Cipher { key });
        }
    }
    let mut key = [0u8; 32];
    rand::fill(&mut key);
    write_key(&path, &key)?;
    Ok(Cipher { key })
}

fn decode_key(raw: &[u8]) -> Result<[u8; 32], ()> {
    let text = std::str::from_utf8(raw).unwrap_or("").trim();
    if text.len() == 64 {
        if let Ok(bytes) = hex::decode(text) {
            if bytes.len() == 32 {
                let mut key = [0u8; 32];
                key.copy_from_slice(&bytes);
                return Ok(key);
            }
        }
    }
    if raw.len() == 32 {
        let mut key = [0u8; 32];
        key.copy_from_slice(raw);
        return Ok(key);
    }
    Err(())
}

fn write_key(path: &Path, key: &[u8; 32]) -> AppResult<()> {
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts
        .open(path)
        .map_err(|e| ApiError::Internal(format!("secret.key: {e}")))?;
    file.write_all(hex::encode(key).as_bytes())
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(())
}

pub fn hash_password(password: &str) -> String {
    let mut salt = [0u8; 16];
    rand::fill(&mut salt);
    let mut out = [0u8; 32];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, PBKDF2_ITERS, &mut out);
    format!(
        "pbkdf2${PBKDF2_ITERS}${}${}",
        hex::encode(salt),
        hex::encode(out)
    )
}

pub fn verify_password(stored: &str, password: &str) -> bool {
    let mut parts = stored.split('$');
    if parts.next() != Some("pbkdf2") {
        return false;
    }
    let Some(iters) = parts.next().and_then(|v| v.parse::<u32>().ok()) else {
        return false;
    };
    let Some(salt) = parts.next().and_then(|v| hex::decode(v).ok()) else {
        return false;
    };
    let Some(expected) = parts.next().and_then(|v| hex::decode(v).ok()) else {
        return false;
    };
    if iters == 0 || iters > 2_000_000 || expected.is_empty() {
        return false;
    }
    let mut out = vec![0u8; expected.len()];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, iters, &mut out);
    use subtle::ConstantTimeEq;
    bool::from(out.ct_eq(&expected))
}

pub fn random_token() -> String {
    let mut buf = [0u8; 32];
    rand::fill(&mut buf);
    hex::encode(buf)
}

pub fn random_password() -> String {
    let mut buf = [0u8; 18];
    rand::fill(&mut buf);
    hex::encode(buf)
}

pub fn tokens_equal(left: &str, right: &str) -> bool {
    use subtle::ConstantTimeEq;
    if left.len() != right.len() {
        return false;
    }
    bool::from(left.as_bytes().ct_eq(right.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_roundtrip() {
        let stored = hash_password("correct horse");
        assert!(verify_password(&stored, "correct horse"));
        assert!(!verify_password(&stored, "nope"));
        assert!(!verify_password("garbage", "correct horse"));
    }

    #[test]
    fn secret_roundtrip() {
        let cipher = Cipher { key: [7u8; 32] };
        let packed = cipher.encrypt("ssh-key").unwrap();
        assert_eq!(cipher.decrypt(&packed).unwrap(), "ssh-key");
        assert!(cipher.decrypt("AAAA").is_err());
    }
}
