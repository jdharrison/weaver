//! Bounded, non-logging reads of operator-provided remote connection material.

use crate::{WovenAdapterError, WovenConfig};
use std::fs::OpenOptions;
use std::io::Read;
use std::path::Path;
use woven_client::ClientTlsConfig;

const MAX_CA_BYTES: usize = 1024 * 1024;
const MAX_TOKEN_FILE_BYTES: usize = 4098;

fn invalid(message: &str) -> WovenAdapterError {
    WovenAdapterError::InitializationFailed(message.to_owned())
}

pub(crate) fn load(config: &WovenConfig) -> Result<(ClientTlsConfig, String), WovenAdapterError> {
    let ca_path = config
        .ca_pem_file
        .as_deref()
        .ok_or_else(|| invalid("missing CA PEM file"))?;
    let token_path = config
        .token_file
        .as_deref()
        .ok_or_else(|| invalid("missing token file"))?;
    let pem = read_bounded(ca_path, MAX_CA_BYTES, false)?;
    let tls = ClientTlsConfig::from_ca_pem(&pem)
        .map_err(|_| invalid("invalid or empty CA PEM bundle"))?;
    let bytes = read_bounded(token_path, MAX_TOKEN_FILE_BYTES, true)?;
    Ok((tls, validate_token(&bytes)?.to_owned()))
}

fn validate_token(bytes: &[u8]) -> Result<&str, WovenAdapterError> {
    let token = std::str::from_utf8(bytes)
        .map_err(|_| invalid("token file must be UTF-8"))?
        .trim_end_matches(['\r', '\n']);
    if !(32..=4096).contains(&token.len()) || !token.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(invalid(
            "token must contain 32–4096 non-whitespace ASCII bytes",
        ));
    }
    Ok(token)
}

fn read_bounded(path: &Path, limit: usize, secret: bool) -> Result<Vec<u8>, WovenAdapterError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A FIFO must not block before we can inspect its metadata.
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|_| invalid("cannot open remote CA/token file"))?;
    let metadata = file
        .metadata()
        .map_err(|_| invalid("cannot inspect remote CA/token file"))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > limit as u64 {
        return Err(invalid(
            "remote CA/token file must be a nonempty bounded regular file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if secret && metadata.permissions().mode() & 0o077 != 0 {
            return Err(invalid("token file must deny group/other permissions"));
        }
    }
    #[cfg(not(unix))]
    let _ = secret;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid("cannot read remote CA/token file"))?;
    if bytes.is_empty() || bytes.len() > limit {
        return Err(invalid(
            "remote CA/token file must be nonempty and within its byte limit",
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_validation_matches_remote_server() {
        let token = "x".repeat(32);
        assert_eq!(
            validate_token(format!("{token}\r\n").as_bytes()).unwrap(),
            token
        );
        assert!(validate_token("x".repeat(4096).as_bytes()).is_ok());
        for bytes in [
            b"".to_vec(),
            b"dev-token".to_vec(),
            vec![b'x'; 4097],
            vec![255; 32],
            vec![b' '; 32],
            format!("{token}\n{token}").into_bytes(),
        ] {
            assert!(validate_token(&bytes).is_err());
        }
    }

    #[test]
    fn bounded_files_and_permissions() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "weaver-remote-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path).unwrap();
        assert!(read_bounded(&path, 32, true).is_err());
        file.set_len(33).unwrap();
        assert!(read_bounded(&path, 32, true).is_err());
        file.set_len(32).unwrap();
        assert_eq!(read_bounded(&path, 32, true).unwrap().len(), 32);
        let config = WovenConfig {
            mode: crate::ConnectivityMode::RemoteQuic,
            endpoint: Some("quic://127.0.0.1:1".into()),
            ca_pem_file: Some(path.clone()),
            token_file: Some(path.clone()),
            ..WovenConfig::default()
        };
        let mut adapter = crate::WovenAdapter::new(config).unwrap();
        // Invalid CA material must fail before any socket connection or dev fallback.
        assert!(adapter.start().unwrap_err().to_string().contains("CA PEM"));
        assert_eq!(adapter.status(), crate::WovenStatus::Stopped);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o644))
                .unwrap();
            assert!(read_bounded(&path, 32, true).is_err());
            assert!(read_bounded(&path, 32, false).is_ok());
        }
        file.set_len(MAX_CA_BYTES as u64 + 1).unwrap();
        assert!(read_bounded(&path, MAX_CA_BYTES, false).is_err());
        drop(file);
        std::fs::remove_file(&path).unwrap();
        assert!(read_bounded(&path, 32, true).is_err());
        assert!(read_bounded(&std::env::temp_dir(), 32, false).is_err());
    }
}
