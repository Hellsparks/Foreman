//! TOFU host-key pinning against an in-process russh SSH server.

use std::sync::Arc;

use foreman_core::ssh_client::{self, ConnectError};
use russh::keys::{Algorithm, PrivateKey};
use russh::server::{Auth, Handler};

struct AcceptAll;

impl Handler for AcceptAll {
    type Error = russh::Error;

    async fn auth_publickey(
        &mut self,
        _user: &str,
        _key: russh::server::VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
    }
}

fn new_key() -> PrivateKey {
    let mut rng = rand_core::UnwrapErr(getrandom::SysRng);
    PrivateKey::random(&mut rng, Algorithm::Ed25519).expect("key")
}

fn fingerprint(k: &PrivateKey) -> String {
    k.public_key()
        .fingerprint(russh::keys::HashAlg::Sha256)
        .to_string()
}

/// Start a server that accepts connections forever with the given host key.
async fn start_server(host_key: PrivateKey) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let config = Arc::new(russh::server::Config {
        keys: vec![host_key],
        auth_rejection_time: std::time::Duration::from_millis(0),
        ..Default::default()
    });
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let cfg = Arc::clone(&config);
            tokio::spawn(async move {
                if let Ok(session) = russh::server::run_stream(cfg, sock, AcceptAll).await {
                    let _ = session.await;
                }
            });
        }
    });
    port
}

#[tokio::test]
async fn first_connect_pins_then_same_key_ok_then_changed_key_refused() {
    let key_a = new_key();
    let fp_a = fingerprint(&key_a);
    let port_a = start_server(key_a).await;

    // First connect: nothing pinned, accepted, fingerprint reported for pinning.
    let mut conn = ssh_client::connect("127.0.0.1", port_a, None)
        .await
        .expect("first connect");
    assert_eq!(conn.received_fingerprint().as_deref(), Some(fp_a.as_str()));
    let client_key = Arc::new(new_key());
    ssh_client::auth_with_key(&mut conn, "pi", Arc::clone(&client_key))
        .await
        .expect("key auth");
    conn.disconnect().await;

    // Same server, pinned key matches.
    let conn = ssh_client::connect("127.0.0.1", port_a, Some(&fp_a))
        .await
        .expect("pinned connect");
    conn.disconnect().await;

    // Server with a different host key: refused, new fingerprint reported.
    let key_b = new_key();
    let fp_b = fingerprint(&key_b);
    let port_b = start_server(key_b).await;
    match ssh_client::connect("127.0.0.1", port_b, Some(&fp_a)).await {
        Err(ConnectError::HostKeyMismatch { presented }) => assert_eq!(presented, fp_b),
        Err(e) => panic!("expected HostKeyMismatch, got {e}"),
        Ok(_) => panic!("changed host key must be refused"),
    }
}
