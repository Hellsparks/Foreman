//! Capture tool: connect to a real Bambu printer and record report messages.
//!
//! Run manually with:
//!   cargo test -p foreman-core --test capture_bambu -- --ignored --nocapture
//!
//! Requires test-printers.local.toml at workspace root with real credentials.
//! Captures ~2 min of reports, strips serial, saves to tests/fixtures/.

use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS, TlsConfiguration, Transport};

fn load_config() -> Option<(String, String, String)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .parent()?
        .join("test-printers.local.toml");
    let content = std::fs::read_to_string(&path).ok()?;
    let table: toml::Table = content.parse().ok()?;
    let bambu = table.get("bambu-a1")?.as_table()?;
    let host = bambu.get("host")?.as_str()?.to_owned();
    let serial = bambu.get("serial")?.as_str()?.to_owned();
    let access_code = bambu.get("access_code")?.as_str()?.to_owned();
    if serial == "REDACTED" || access_code == "REDACTED" {
        return None;
    }
    Some((host, serial, access_code))
}

/// Accept-any TLS config for Bambu self-signed cert.
fn tls_config() -> TlsConfiguration {
    use rumqttc::tokio_rustls::rustls::ClientConfig;

    #[derive(Debug)]
    struct AcceptAny;

    impl rumqttc::tokio_rustls::rustls::client::danger::ServerCertVerifier for AcceptAny {
        fn verify_server_cert(
            &self,
            _end_entity: &rumqttc::tokio_rustls::rustls::pki_types::CertificateDer<'_>,
            _intermediates: &[rumqttc::tokio_rustls::rustls::pki_types::CertificateDer<'_>],
            _server_name: &rumqttc::tokio_rustls::rustls::pki_types::ServerName<'_>,
            _ocsp_response: &[u8],
            _now: rumqttc::tokio_rustls::rustls::pki_types::UnixTime,
        ) -> Result<
            rumqttc::tokio_rustls::rustls::client::danger::ServerCertVerified,
            rumqttc::tokio_rustls::rustls::Error,
        > {
            Ok(rumqttc::tokio_rustls::rustls::client::danger::ServerCertVerified::assertion())
        }
        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &rumqttc::tokio_rustls::rustls::pki_types::CertificateDer<'_>,
            _dss: &rumqttc::tokio_rustls::rustls::DigitallySignedStruct,
        ) -> Result<
            rumqttc::tokio_rustls::rustls::client::danger::HandshakeSignatureValid,
            rumqttc::tokio_rustls::rustls::Error,
        > {
            Ok(rumqttc::tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
        }
        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &rumqttc::tokio_rustls::rustls::pki_types::CertificateDer<'_>,
            _dss: &rumqttc::tokio_rustls::rustls::DigitallySignedStruct,
        ) -> Result<
            rumqttc::tokio_rustls::rustls::client::danger::HandshakeSignatureValid,
            rumqttc::tokio_rustls::rustls::Error,
        > {
            Ok(rumqttc::tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
        }
        fn supported_verify_schemes(&self) -> Vec<rumqttc::tokio_rustls::rustls::SignatureScheme> {
            use rumqttc::tokio_rustls::rustls::SignatureScheme;
            vec![
                SignatureScheme::RSA_PKCS1_SHA256,
                SignatureScheme::RSA_PKCS1_SHA384,
                SignatureScheme::RSA_PKCS1_SHA512,
                SignatureScheme::ECDSA_NISTP256_SHA256,
                SignatureScheme::ECDSA_NISTP384_SHA384,
                SignatureScheme::ECDSA_NISTP521_SHA512,
                SignatureScheme::RSA_PSS_SHA256,
                SignatureScheme::RSA_PSS_SHA384,
                SignatureScheme::RSA_PSS_SHA512,
                SignatureScheme::ED25519,
            ]
        }
    }

    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAny))
        .with_no_client_auth();
    TlsConfiguration::Rustls(Arc::new(config))
}

#[tokio::test]
#[ignore]
async fn capture_a1_reports() {
    let (host, serial, access_code) = match load_config() {
        Some(c) => c,
        None => {
            eprintln!("Skipping: test-printers.local.toml not configured");
            return;
        }
    };

    let report_topic = format!("device/{serial}/report");
    let request_topic = format!("device/{serial}/request");

    let mut opts = MqttOptions::new("foreman-capture", &host, 8883);
    opts.set_credentials("bblp", &access_code);
    opts.set_keep_alive(Duration::from_secs(30));
    opts.set_transport(Transport::tls_with_config(tls_config()));

    let (client, mut eventloop) = AsyncClient::new(opts, 64);

    client
        .subscribe(&report_topic, QoS::AtMostOnce)
        .await
        .expect("subscribe");

    // Send pushall.
    let pushall = serde_json::json!({
        "pushing": {
            "sequence_id": "0",
            "command": "pushall",
            "version": 1,
            "push_target": 1
        }
    });
    client
        .publish(
            &request_topic,
            QoS::AtMostOnce,
            false,
            serde_json::to_vec(&pushall).unwrap(),
        )
        .await
        .expect("pushall");

    println!("Connected to {host}, capturing for ~2 minutes...");

    let mut messages: Vec<serde_json::Value> = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);

    loop {
        tokio::select! {
            event = eventloop.poll() => {
                match event {
                    Ok(Event::Incoming(Packet::Publish(publish))) => {
                        if publish.topic == report_topic {
                            if let Ok(mut val) = serde_json::from_slice::<serde_json::Value>(&publish.payload) {
                                // Strip serial, access code, and IP from the payload.
                                strip_secrets(&mut val, &[&serial, &access_code, &host]);
                                messages.push(val);
                                print!("\r  captured {} messages", messages.len());
                                std::io::stdout().flush().ok();
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("\nMQTT error: {e}");
                        break;
                    }
                    _ => {}
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                println!("\n  timeout reached");
                break;
            }
        }
    }

    let _ = client.disconnect().await;

    // Write fixture.
    let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/bambu_a1_captured.json");
    let json = serde_json::to_string_pretty(&messages).expect("serialize");
    std::fs::write(&fixture_path, json).expect("write fixture");
    println!(
        "Wrote {} messages to {}",
        messages.len(),
        fixture_path.display()
    );
}

fn strip_secrets(val: &mut serde_json::Value, secrets: &[&str]) {
    match val {
        serde_json::Value::String(s) => {
            for secret in secrets {
                if s.contains(secret) {
                    *s = s.replace(secret, "REDACTED");
                }
            }
        }
        serde_json::Value::Object(map) => {
            for v in map.values_mut() {
                strip_secrets(v, secrets);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                strip_secrets(v, secrets);
            }
        }
        _ => {}
    }
}
