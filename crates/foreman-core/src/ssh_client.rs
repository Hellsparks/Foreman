//! SSH client helpers using `russh` (+ `russh-sftp` for the SFTP subsystem).
//!
//! Crate choice: `russh` 0.65 / `russh-sftp` 2.4 — pure-Rust, tokio-native, no libssh2/OpenSSL
//! (keeps Windows CI simple). Logged in the doc 05 corrections log.
//!
//! Identity: one Foreman ed25519 key, generated on first start and stored in `setting`
//! encrypted with [`SecretBox`]. Host keys are pinned per printer on first connect (TOFU).

use std::sync::{Arc, Mutex};

use sqlx::SqlitePool;

use crate::{error::AppError, secrets::SecretBox};

// ---------------------------------------------------------------------------
// TOFU-pinning handler
// ---------------------------------------------------------------------------

/// What the handler saw during key exchange.
#[derive(Default)]
pub struct HandlerShared {
    pub received_fingerprint: Option<String>,
    pub host_key_accepted: bool,
}

/// russh client handler implementing TOFU host-key pinning.
pub struct ClientHandler {
    pub expected_fingerprint: Option<String>,
    pub shared: Arc<Mutex<HandlerShared>>,
}

#[derive(Debug, thiserror::Error)]
pub enum SshHandlerError {
    #[error("russh: {0}")]
    Russh(#[from] russh::Error),
    #[error("mutex poisoned")]
    Mutex,
}

impl russh::client::Handler for ClientHandler {
    type Error = SshHandlerError;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let fp = server_public_key
            .public_key()
            .fingerprint(russh::keys::HashAlg::Sha256)
            .to_string();
        let mut shared = self.shared.lock().map_err(|_| SshHandlerError::Mutex)?;
        let accept = match &self.expected_fingerprint {
            None => true,
            Some(expected) => expected == &fp,
        };
        shared.received_fingerprint = Some(fp);
        shared.host_key_accepted = accept;
        Ok(accept)
    }
}

// ---------------------------------------------------------------------------
// Connection
// ---------------------------------------------------------------------------

/// An SSH session (after handshake; authenticate before use).
pub struct SshConnection {
    pub handle: russh::client::Handle<ClientHandler>,
    shared: Arc<Mutex<HandlerShared>>,
}

impl SshConnection {
    /// SHA-256 fingerprint (`SHA256:…`) of the server host key.
    pub fn received_fingerprint(&self) -> Option<String> {
        self.shared
            .lock()
            .ok()
            .and_then(|s| s.received_fingerprint.clone())
    }

    pub async fn disconnect(&self) {
        let _ = self
            .handle
            .disconnect(russh::Disconnect::ByApplication, "", "English")
            .await;
    }
}

/// Connect failure: either a host-key change (needs user approval) or anything else.
#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("SSH host key changed (server presented {presented})")]
    HostKeyMismatch { presented: String },
    #[error(transparent)]
    Other(#[from] AppError),
}

/// TCP connect + SSH handshake. With `expected_fingerprint = None` any key is accepted
/// (first connect, caller pins it); otherwise a different key is refused.
pub async fn connect(
    host: &str,
    port: u16,
    expected_fingerprint: Option<&str>,
) -> Result<SshConnection, ConnectError> {
    let shared = Arc::new(Mutex::new(HandlerShared::default()));
    let handler = ClientHandler {
        expected_fingerprint: expected_fingerprint.map(str::to_owned),
        shared: Arc::clone(&shared),
    };
    let config = Arc::new(russh::client::Config {
        inactivity_timeout: Some(std::time::Duration::from_secs(600)),
        keepalive_interval: Some(std::time::Duration::from_secs(30)),
        ..Default::default()
    });

    let fut = russh::client::connect(config, (host, port), handler);
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), fut)
        .await
        .map_err(|_| AppError::Driver(format!("SSH connect to {host}:{port}: timed out")))?;

    match result {
        Ok(handle) => Ok(SshConnection { handle, shared }),
        Err(e) => {
            let (fp, accepted) = shared
                .lock()
                .map(|s| (s.received_fingerprint.clone(), s.host_key_accepted))
                .unwrap_or((None, true));
            match fp {
                Some(presented) if !accepted => Err(ConnectError::HostKeyMismatch { presented }),
                _ => Err(AppError::Driver(format!("SSH connect to {host}:{port}: {e}")).into()),
            }
        }
    }
}

/// Public-key auth with the Foreman identity.
pub async fn auth_with_key(
    conn: &mut SshConnection,
    user: &str,
    key: Arc<russh::keys::PrivateKey>,
) -> Result<(), AppError> {
    let hash = conn
        .handle
        .best_supported_rsa_hash()
        .await
        .ok()
        .flatten()
        .flatten();
    let res = conn
        .handle
        .authenticate_publickey(user, russh::keys::PrivateKeyWithHashAlg::new(key, hash))
        .await
        .map_err(|e| AppError::Driver(format!("SSH key auth: {e}")))?;
    if !res.success() {
        return Err(AppError::Driver(format!(
            "SSH key auth rejected for '{user}' — add Foreman's public key to ~/.ssh/authorized_keys"
        )));
    }
    Ok(())
}

/// Password auth.
pub async fn auth_with_password(
    conn: &mut SshConnection,
    user: &str,
    password: &str,
) -> Result<(), AppError> {
    let res = conn
        .handle
        .authenticate_password(user, password)
        .await
        .map_err(|e| AppError::Driver(format!("SSH password auth: {e}")))?;
    if !res.success() {
        return Err(AppError::Driver(format!(
            "SSH password auth rejected for '{user}'"
        )));
    }
    Ok(())
}

/// Run one command on an exec channel; returns `(stdout, stderr, exit_code)`.
pub async fn exec_command(
    conn: &SshConnection,
    command: &str,
) -> Result<(Vec<u8>, Vec<u8>, u32), AppError> {
    let mut channel = conn
        .handle
        .channel_open_session()
        .await
        .map_err(|e| AppError::Driver(format!("channel open: {e}")))?;
    channel
        .exec(true, command)
        .await
        .map_err(|e| AppError::Driver(format!("exec: {e}")))?;

    let (mut stdout, mut stderr, mut code) = (Vec::new(), Vec::new(), 0u32);
    while let Some(msg) = channel.wait().await {
        match msg {
            russh::ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
            russh::ChannelMsg::ExtendedData { data, ext: 1 } => stderr.extend_from_slice(&data),
            russh::ChannelMsg::ExitStatus { exit_status } => code = exit_status,
            _ => {}
        }
    }
    Ok((stdout, stderr, code))
}

/// Open the SFTP subsystem.
pub async fn open_sftp(conn: &SshConnection) -> Result<russh_sftp::client::SftpSession, AppError> {
    let channel = conn
        .handle
        .channel_open_session()
        .await
        .map_err(|e| AppError::Driver(format!("sftp channel open: {e}")))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| AppError::Driver(format!("sftp subsystem: {e}")))?;
    russh_sftp::client::SftpSession::new(channel.into_stream())
        .await
        .map_err(|e| AppError::Driver(format!("sftp init: {e}")))
}

/// Open an interactive shell on a PTY.
pub async fn open_pty_shell(
    conn: &SshConnection,
    cols: u32,
    rows: u32,
) -> Result<russh::Channel<russh::client::Msg>, AppError> {
    let channel = conn
        .handle
        .channel_open_session()
        .await
        .map_err(|e| AppError::Driver(format!("pty channel open: {e}")))?;
    channel
        .request_pty(true, "xterm-256color", cols, rows, 0, 0, &[])
        .await
        .map_err(|e| AppError::Driver(format!("request pty: {e}")))?;
    channel
        .request_shell(true)
        .await
        .map_err(|e| AppError::Driver(format!("request shell: {e}")))?;
    Ok(channel)
}

// ---------------------------------------------------------------------------
// Host from a connection-path URL
// ---------------------------------------------------------------------------

/// Extract the host from a path URL (`http://h:7125/x`, `h:7125`, `h`, `[fe80::1]:7125`).
pub fn host_from_url(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next()?
    } else {
        authority.split(':').next()?
    };
    (!host.is_empty()).then(|| host.to_owned())
}

// ---------------------------------------------------------------------------
// Foreman SSH identity (setting table, encrypted)
// ---------------------------------------------------------------------------

const SETTING_PRIVATE_KEY: &str = "ssh.private_key";
const SETTING_PRIVATE_KEY_LEGACY: &str = "ssh.private_key_pem";
const SETTING_PUBLIC_KEY: &str = "ssh.public_key";

async fn get_setting(pool: &SqlitePool, key: &str) -> Result<Option<String>, AppError> {
    sqlx::query_scalar("SELECT value FROM setting WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))
}

async fn put_setting(pool: &SqlitePool, key: &str, value: &str) -> Result<(), AppError> {
    sqlx::query("INSERT OR REPLACE INTO setting (key, value, updated_at) VALUES (?, ?, ?)")
        .bind(key)
        .bind(value)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(())
}

/// Load (or generate once) the Foreman ed25519 identity.
/// Returns `(private key, "ssh-ed25519 AAAA… foreman")`.
pub async fn ensure_identity(
    pool: &SqlitePool,
    secrets: &SecretBox,
) -> Result<(Arc<russh::keys::PrivateKey>, String), AppError> {
    let pem = match get_setting(pool, SETTING_PRIVATE_KEY).await? {
        Some(sealed) => secrets.open(&sealed)?,
        None => {
            let pem = match get_setting(pool, SETTING_PRIVATE_KEY_LEGACY).await? {
                // Pre-release dev builds stored the key in plaintext; re-seal it.
                Some(plain) => plain,
                None => {
                    let mut rng = rand_core::UnwrapErr(getrandom::SysRng);
                    let kp =
                        russh::keys::PrivateKey::random(&mut rng, russh::keys::Algorithm::Ed25519)
                            .map_err(|e| AppError::Internal(format!("generate SSH key: {e}")))?;
                    let mut buf = Vec::new();
                    russh::keys::encode_pkcs8_pem(&kp, &mut buf)
                        .map_err(|e| AppError::Internal(format!("encode SSH key: {e}")))?;
                    String::from_utf8(buf)
                        .map_err(|e| AppError::Internal(format!("SSH key utf-8: {e}")))?
                }
            };
            put_setting(pool, SETTING_PRIVATE_KEY, &secrets.seal(&pem)?).await?;
            sqlx::query("DELETE FROM setting WHERE key = ?")
                .bind(SETTING_PRIVATE_KEY_LEGACY)
                .execute(pool)
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;
            pem
        }
    };

    let key = russh::keys::decode_secret_key(&pem, None)
        .map_err(|e| AppError::Internal(format!("decode SSH key: {e}")))?;
    let public = format!(
        "{} {} foreman",
        key.public_key().algorithm().as_str(),
        russh::keys::PublicKeyBase64::public_key_base64(key.public_key())
    );
    put_setting(pool, SETTING_PUBLIC_KEY, &public).await?;
    Ok((Arc::new(key), public))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use russh::client::Handler as _;

    fn server_key() -> russh::keys::PrivateKey {
        let mut rng = rand_core::UnwrapErr(getrandom::SysRng);
        russh::keys::PrivateKey::random(&mut rng, russh::keys::Algorithm::Ed25519)
            .expect("generate test key")
    }

    fn handler(expected: Option<String>) -> (ClientHandler, Arc<Mutex<HandlerShared>>) {
        let shared = Arc::new(Mutex::new(HandlerShared::default()));
        (
            ClientHandler {
                expected_fingerprint: expected,
                shared: Arc::clone(&shared),
            },
            shared,
        )
    }

    #[tokio::test]
    async fn handler_accepts_first_and_matching_rejects_changed() {
        let key = server_key();
        let pk = russh::keys::PublicKeyOrCertificate::from(key.public_key().clone());
        let fp = key
            .public_key()
            .fingerprint(russh::keys::HashAlg::Sha256)
            .to_string();

        let (mut h, s) = handler(None);
        assert!(h.check_server_key(&pk).await.expect("check"));
        assert_eq!(
            s.lock().expect("lock").received_fingerprint.as_deref(),
            Some(fp.as_str())
        );

        let (mut h, _) = handler(Some(fp));
        assert!(h.check_server_key(&pk).await.expect("check"));

        let (mut h, s) = handler(Some("SHA256:something-else".into()));
        assert!(!h.check_server_key(&pk).await.expect("check"));
        assert!(!s.lock().expect("lock").host_key_accepted);
    }

    #[test]
    fn host_from_url_variants() {
        assert_eq!(
            host_from_url("http://10.0.0.177:7125").as_deref(),
            Some("10.0.0.177")
        );
        assert_eq!(
            host_from_url("https://voron.local/api").as_deref(),
            Some("voron.local")
        );
        assert_eq!(host_from_url("10.0.0.5").as_deref(), Some("10.0.0.5"));
        assert_eq!(host_from_url("host:22").as_deref(), Some("host"));
        assert_eq!(
            host_from_url("http://[fe80::1]:7125").as_deref(),
            Some("fe80::1")
        );
        assert_eq!(host_from_url("http://").as_deref(), None);
    }

    #[tokio::test]
    async fn identity_generated_once_and_sealed() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let db = crate::db::ForemanDb::connect(&dir.path().join("f.db"))
            .await
            .expect("db");
        let sb = SecretBox::from_key([9u8; 32]);
        let (_, pub1) = ensure_identity(&db.pool, &sb).await.expect("first");
        let (_, pub2) = ensure_identity(&db.pool, &sb).await.expect("second");
        assert_eq!(pub1, pub2);
        assert!(pub1.starts_with("ssh-ed25519 "));
        let stored = get_setting(&db.pool, SETTING_PRIVATE_KEY)
            .await
            .expect("get")
            .expect("present");
        assert!(SecretBox::is_sealed(&stored));
        assert!(!stored.contains("PRIVATE KEY"));
    }
}
