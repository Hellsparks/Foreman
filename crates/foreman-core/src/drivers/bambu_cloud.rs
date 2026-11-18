//! Bambu Cloud transport, monitoring only (doc 07 §B, session "Cloud transport").
//!
//! [`CloudHub`] holds **one MQTT connection per Bambu account**, shared by every printer
//! linked to that account. It subscribes to `device/<serial>/report` for each linked
//! printer (fully specified topics, no wildcards) and merges every report into a
//! per-printer [`CachedStatus`], the same parser the LAN driver uses.
//!
//! [`BambuCloudDriver`] is the printer's cloud *path*: it reads that merged state and
//! publishes it like any driver. It never publishes to `device/<serial>/request`; there is
//! no dispatch, upload, camera or `pushall` here. Everything that would touch the printer
//! is refused by the compatibility engine before it gets here ([`crate::compat::Reach`]);
//! the driver's own refusals are only a backstop.
//!
//! Credentials differ from LAN in every field:
//! - broker `us.mqtt.bambulab.com:8883` (China: `cn.mqtt.bambulab.com`), TLS with the
//!   platform's root certificates;
//! - username `u_<uid>`: the JWT's `preferred_username` when the token is a JWT, else the
//!   `uid` from `GET /v1/user-service/my/profile` (current tokens are opaque; see doc 04
//!   corrections log);
//! - password: the access token.
//!
//! Token expiry: a broker refusal (`BadUserNamePassword` / `NotAuthorized`) or a 401 from
//! the profile call marks the account's token rejected, stops that account's connection
//! (no retry, no login call), and raises one Attention item naming the account. Only that
//! account is touched. Network failures back off (2 s → 60 s) and retry.
//!
//! Without `pushall`, P1/A1 printers send deltas: a report may not say whether the printer
//! is printing. The merged state stays "unknown" until a report carries `gcode_state` or
//! another path seeds it ([`CloudHub::seed`]); the driver publishes nothing until then,
//! so an unknown state can never close a running job.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use rumqttc::{
    AsyncClient, ConnectReturnCode, ConnectionError, Event, MqttOptions, Packet, QoS,
    TlsConfiguration, Transport,
};
use serde::Serialize;
use serde_json::Value;
use sqlx::SqlitePool;
use tokio::sync::{broadcast, watch, Mutex, RwLock};
use tracing::{debug, info, warn};
use ts_rs::TS;

use super::bambu::{caps_from_serial, CachedStatus};
use crate::bambu_cloud::{BambuCloud, CloudError, Refusal, Region, Token};
use crate::driver::{PrinterCapabilities, PrinterCommand, PrinterDriver, PrinterFile};
use crate::error::AppError;
use crate::events::{
    AttentionItemEvent, ConnectionEvent, ConnectionEventKind, PrinterState, PrinterStatus,
    ServerEvent,
};
use crate::repo::bambu_account;
use crate::secrets::SecretBox;

/// URL of a printer's cloud path: `bambu-cloud://<account id>`. Never stored by the user;
/// the driver manager adds it as the last path of every printer linked to an account.
pub const CLOUD_URL_PREFIX: &str = "bambu-cloud://";

/// Display name of the cloud path ("via Bambu Cloud").
pub const CLOUD_PATH_NAME: &str = "Bambu Cloud";

/// A cloud path that saw no report for this long counts as down.
const REPORT_STALE_AFTER: Duration = Duration::from_secs(120);

/// Largest MQTT packet accepted. A full P1/X1 report with AMS is tens of kB, above
/// rumqttc's 10 kB default.
const MAX_PACKET: usize = 512 * 1024;

pub fn cloud_path_url(account_id: i64) -> String {
    format!("{CLOUD_URL_PREFIX}{account_id}")
}

/// The account id of a cloud path URL, or None for any other URL.
pub fn cloud_account_of(url: &str) -> Option<i64> {
    url.strip_prefix(CLOUD_URL_PREFIX)?.parse().ok()
}

fn broker(region: Region) -> &'static str {
    match region {
        Region::Global => "us.mqtt.bambulab.com",
        Region::China => "cn.mqtt.bambulab.com",
    }
}

/// Attention kind for "re-authenticate this account" (one open item per account).
pub fn auth_attention_kind(account_id: i64) -> String {
    format!("bambu_auth:{account_id}")
}

/// Attention kind for "Bambu refused a request (HTTP 403)" (one open item per account;
/// a later refusal replaces its text).
pub fn refused_attention_kind(account_id: i64) -> String {
    format!("bambu_refused:{account_id}")
}

/// `u_<uid>` from a JWT's `preferred_username` claim, when the token is a JWT.
pub fn jwt_mqtt_username(token: &Token) -> Option<String> {
    use base64::Engine as _;
    let payload = token.expose().split('.').nth(1)?;
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let v: Value = serde_json::from_slice(&raw).ok()?;
    let name = v.get("preferred_username")?.as_str()?;
    if name.is_empty() {
        return None;
    }
    Some(if name.starts_with("u_") {
        name.to_owned()
    } else {
        format!("u_{name}")
    })
}

// ---------------------------------------------------------------------------
// Session state
// ---------------------------------------------------------------------------

/// Where one account's cloud connection stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SessionState {
    Connecting,
    Connected,
    /// No token stored: nothing to connect with.
    SignedOut,
    /// Bambu refused the token. Stopped until the account is re-authenticated.
    AuthExpired {
        reason: String,
    },
    /// Network / broker trouble; retrying with backoff.
    Down {
        reason: String,
    },
}

/// What an account's cloud connection has heard from one printer.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct FeedInfo {
    #[ts(type = "number")]
    pub reports: u64,
    #[ts(type = "number | null")]
    pub last_report_secs: Option<u64>,
    /// A report said whether it is printing (or LAN told us).
    pub state_known: bool,
}

/// Why a connection attempt ended.
enum Ended {
    Shutdown,
    Auth(String),
    Net(String),
}

/// Reports from one printer, merged.
#[derive(Default)]
struct Feed {
    status: CachedStatus,
    last_report: Option<Instant>,
    reports: u64,
}

struct Session {
    account_id: i64,
    serials: Mutex<HashSet<String>>,
    feeds: RwLock<HashMap<String, Feed>>,
    state: watch::Sender<SessionState>,
    /// Bumped on every report (drivers check their own serial's feed).
    seq: watch::Sender<u64>,
    client: Mutex<Option<AsyncClient>>,
    shutdown: watch::Sender<bool>,
}

impl Session {
    fn new(account_id: i64) -> Self {
        Self {
            account_id,
            serials: Mutex::new(HashSet::new()),
            feeds: RwLock::new(HashMap::new()),
            state: watch::channel(SessionState::Connecting).0,
            seq: watch::channel(0).0,
            client: Mutex::new(None),
            shutdown: watch::channel(false).0,
        }
    }
}

// ---------------------------------------------------------------------------
// Hub
// ---------------------------------------------------------------------------

/// One cloud MQTT connection per Bambu account, shared by that account's printers.
pub struct CloudHub {
    pool: SqlitePool,
    secrets: Arc<SecretBox>,
    http: BambuCloud,
    bus_tx: Arc<broadcast::Sender<ServerEvent>>,
    sessions: Mutex<HashMap<i64, Arc<Session>>>,
}

impl CloudHub {
    pub fn new(
        pool: SqlitePool,
        secrets: Arc<SecretBox>,
        http: BambuCloud,
        bus_tx: Arc<broadcast::Sender<ServerEvent>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            secrets,
            http,
            bus_tx,
            sessions: Mutex::new(HashMap::new()),
        })
    }

    /// Bring the connections in line with the database: one per account that has linked
    /// printers, subscribed to exactly those printers' report topics. Accounts without
    /// printers are disconnected.
    pub async fn sync(self: &Arc<Self>) -> Result<(), AppError> {
        let rows: Vec<(i64, String)> = sqlx::query_as(
            "SELECT bambu_account_id, serial_number FROM printer \
             WHERE bambu_account_id IS NOT NULL AND serial_number IS NOT NULL \
             AND serial_number != ''",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;
        let mut wanted: HashMap<i64, HashSet<String>> = HashMap::new();
        for (acct, serial) in rows {
            wanted.entry(acct).or_default().insert(serial);
        }

        let mut sessions = self.sessions.lock().await;
        // Stop sessions for accounts with no printers left.
        let gone: Vec<i64> = sessions
            .keys()
            .filter(|a| !wanted.contains_key(a))
            .copied()
            .collect();
        for acct in gone {
            if let Some(s) = sessions.remove(&acct) {
                let _ = s.shutdown.send_replace(true);
                info!(
                    account_id = acct,
                    "bambu cloud: no linked printers; disconnecting"
                );
            }
        }
        for (acct, serials) in wanted {
            let session = match sessions.get(&acct) {
                Some(s) => Arc::clone(s),
                None => {
                    let s = Arc::new(Session::new(acct));
                    sessions.insert(acct, Arc::clone(&s));
                    *s.serials.lock().await = serials.clone();
                    self.spawn(Arc::clone(&s));
                    continue;
                }
            };
            let mut have = session.serials.lock().await;
            let added: Vec<String> = serials.difference(&have).cloned().collect();
            *have = serials;
            drop(have);
            let client = session.client.lock().await.clone();
            if let Some(client) = client {
                for serial in added {
                    let topic = format!("device/{serial}/report");
                    if let Err(e) = client.subscribe(&topic, QoS::AtMostOnce).await {
                        warn!(account_id = acct, "bambu cloud: subscribe {serial}: {e}");
                    }
                }
            }
        }
        Ok(())
    }

    /// The account got a new token: drop its connection and start fresh.
    pub async fn restart(self: &Arc<Self>, account_id: i64) -> Result<(), AppError> {
        if let Some(s) = self.sessions.lock().await.remove(&account_id) {
            let _ = s.shutdown.send_replace(true);
        }
        self.clear_auth_attention(account_id).await;
        self.sync().await
    }

    /// Stop every connection (server shutdown, tests).
    pub async fn shutdown(&self) {
        for (_, s) in self.sessions.lock().await.drain() {
            let _ = s.shutdown.send_replace(true);
        }
    }

    async fn session(&self, account_id: i64) -> Option<Arc<Session>> {
        self.sessions.lock().await.get(&account_id).cloned()
    }

    /// Where an account's connection stands (None = no connection: no linked printers).
    pub async fn state(&self, account_id: i64) -> Option<SessionState> {
        Some(self.session(account_id).await?.state.borrow().clone())
    }

    /// The cloud path to this printer works: the account is connected and the printer
    /// reported recently.
    pub async fn path_healthy(&self, account_id: i64, serial: &str) -> bool {
        let Some(s) = self.session(account_id).await else {
            return false;
        };
        if *s.state.borrow() != SessionState::Connected {
            return false;
        }
        let feeds = s.feeds.read().await;
        feeds
            .get(serial)
            .and_then(|f| f.last_report)
            .is_some_and(|t| t.elapsed() < REPORT_STALE_AFTER)
    }

    /// What the account's connection has heard from this printer.
    pub async fn feed_info(&self, account_id: i64, serial: &str) -> Option<FeedInfo> {
        let s = self.session(account_id).await?;
        let feeds = s.feeds.read().await;
        let f = feeds.get(serial);
        Some(FeedInfo {
            reports: f.map_or(0, |f| f.reports),
            last_report_secs: f.and_then(|f| f.last_report).map(|t| t.elapsed().as_secs()),
            state_known: f.is_some_and(|f| f.status.state_known),
        })
    }

    /// Another path knows the print state (LAN reported it): use it for this printer's
    /// cloud feed until a cloud report carries `gcode_state` itself.
    pub async fn seed(&self, account_id: i64, serial: &str, status: &PrinterStatus) {
        if status.state == PrinterState::Offline {
            return;
        }
        let Some(s) = self.session(account_id).await else {
            return;
        };
        let mut feeds = s.feeds.write().await;
        let feed = feeds.entry(serial.to_owned()).or_default();
        if !feed.status.state_known {
            feed.status.seed_state(status);
            debug!(account_id, serial, state = ?status.state, "bambu cloud: feed seeded");
        }
    }

    fn spawn(self: &Arc<Self>, session: Arc<Session>) {
        let hub = Arc::clone(self);
        tokio::spawn(async move { hub.run(session).await });
    }

    /// One account's connection, until shutdown or a rejected token.
    async fn run(self: Arc<Self>, s: Arc<Session>) {
        let acct = s.account_id;
        let mut shutdown = s.shutdown.subscribe();
        let mut backoff = Duration::from_secs(2);
        // Resolved once per session: reconnects do not call the REST API again.
        let mut username: Option<String> = None;
        loop {
            if *shutdown.borrow() {
                break;
            }
            let _ = s.state.send_replace(SessionState::Connecting);
            let account = match bambu_account::get(&self.pool, acct).await {
                Ok(a) => a,
                Err(e) => {
                    warn!(account_id = acct, "bambu cloud: load account: {e}");
                    break;
                }
            };
            if account.token_rejected_at.is_some() {
                let reason = account
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "token rejected".into());
                let _ = s.state.send_replace(SessionState::AuthExpired { reason });
                info!(
                    account_id = acct,
                    "bambu cloud: token was rejected earlier; not connecting"
                );
                break;
            }
            let token = match bambu_account::get_sealed_token(&self.pool, acct).await {
                Ok(Some(sealed)) => match self.secrets.open(&sealed) {
                    Ok(t) => Token::new(t),
                    Err(e) => {
                        warn!(account_id = acct, "bambu cloud: open token: {e}");
                        let _ = s.state.send_replace(SessionState::SignedOut);
                        break;
                    }
                },
                _ => {
                    let _ = s.state.send_replace(SessionState::SignedOut);
                    break;
                }
            };
            if username.is_none() {
                username = match jwt_mqtt_username(&token) {
                    Some(u) => Some(u),
                    None => match self.http.profile(account.region, &token).await {
                        Ok(p) => p.uid.map(|uid| format!("u_{uid}")),
                        Err(CloudError::Unauthorized(code)) => {
                            self.auth_failed(
                                &s,
                                &account.label,
                                &format!("Bambu rejected the token (HTTP {code})"),
                            )
                            .await;
                            break;
                        }
                        Err(CloudError::Forbidden(r)) => {
                            // Refused, not unreachable: record it as sent, don't retry.
                            self.refused(acct, &r).await;
                            let _ = s.state.send_replace(SessionState::Down {
                                reason: r.to_string(),
                            });
                            break;
                        }
                        Err(e) => {
                            let _ = s.state.send_replace(SessionState::Down {
                                reason: e.to_string(),
                            });
                            warn!(account_id = acct, "bambu cloud: profile: {e}");
                            if !backoff_or_stop(&mut backoff, &mut shutdown).await {
                                break;
                            }
                            continue;
                        }
                    },
                };
                if username.is_none() {
                    let _ = s.state.send_replace(SessionState::Down {
                        reason: "Bambu's profile has no uid".into(),
                    });
                    warn!(
                        account_id = acct,
                        "bambu cloud: profile has no uid; stopping"
                    );
                    break;
                }
            }
            let user = username.clone().unwrap_or_default();
            let started = Instant::now();
            match self
                .connect_once(&s, account.region, &user, &token, &mut shutdown)
                .await
            {
                Ended::Shutdown => break,
                Ended::Auth(reason) => {
                    self.auth_failed(&s, &account.label, &reason).await;
                    break;
                }
                Ended::Net(reason) => {
                    warn!(account_id = acct, "bambu cloud: connection lost: {reason}");
                    let _ = s.state.send_replace(SessionState::Down { reason });
                    // Reports missed while down may include a gcode_state change.
                    for f in s.feeds.write().await.values_mut() {
                        f.status.state_known = false;
                    }
                    if started.elapsed() > Duration::from_secs(120) {
                        backoff = Duration::from_secs(2);
                    }
                    if !backoff_or_stop(&mut backoff, &mut shutdown).await {
                        break;
                    }
                }
            }
        }
        *s.client.lock().await = None;
        info!(account_id = acct, "bambu cloud: connection stopped");
    }

    async fn connect_once(
        &self,
        s: &Session,
        region: Region,
        username: &str,
        token: &Token,
        shutdown: &mut watch::Receiver<bool>,
    ) -> Ended {
        let acct = s.account_id;
        let client_id = format!("foreman-{acct}-{}", uuid::Uuid::new_v4().simple());
        let mut opts = MqttOptions::new(client_id, broker(region), 8883);
        opts.set_credentials(username, token.expose());
        opts.set_keep_alive(Duration::from_secs(30));
        opts.set_max_packet_size(MAX_PACKET, MAX_PACKET);
        let tls = match platform_tls() {
            Ok(t) => t,
            Err(e) => return Ended::Net(e),
        };
        opts.set_transport(Transport::tls_with_config(tls));
        let (client, mut eventloop) = AsyncClient::new(opts, 64);
        eventloop.set_network_options(super::bambu::network_options());
        let serials: Vec<String> = s.serials.lock().await.iter().cloned().collect();
        for serial in &serials {
            if let Err(e) = client
                .subscribe(format!("device/{serial}/report"), QoS::AtMostOnce)
                .await
            {
                return Ended::Net(format!("subscribe: {e}"));
            }
        }
        *s.client.lock().await = Some(client.clone());
        info!(
            account_id = acct,
            printers = serials.len(),
            broker = broker(region),
            "bambu cloud: connecting"
        );

        loop {
            tokio::select! {
                ev = eventloop.poll() => match ev {
                    Ok(Event::Incoming(Packet::ConnAck(_))) => {
                        info!(account_id = acct, "bambu cloud: connected");
                        let _ = s.state.send_replace(SessionState::Connected);
                        if let Err(e) = bambu_account::mark_ok(&self.pool, acct).await {
                            warn!(account_id = acct, "bambu cloud: mark ok: {e}");
                        }
                    }
                    Ok(Event::Incoming(Packet::Publish(p))) => {
                        let Some(serial) = p
                            .topic
                            .strip_prefix("device/")
                            .and_then(|t| t.strip_suffix("/report"))
                        else {
                            continue;
                        };
                        let v: Value = match serde_json::from_slice(&p.payload) {
                            Ok(v) => v,
                            Err(e) => {
                                debug!(account_id = acct, serial, "bambu cloud: bad JSON: {e}");
                                continue;
                            }
                        };
                        let mut feeds = s.feeds.write().await;
                        let feed = feeds.entry(serial.to_owned()).or_default();
                        if let Some(print) = v.get("print") {
                            feed.status.merge_report(print);
                        }
                        feed.last_report = Some(Instant::now());
                        feed.reports += 1;
                        drop(feeds);
                        s.seq.send_modify(|n| *n += 1);
                    }
                    Ok(_) => {}
                    Err(ConnectionError::ConnectionRefused(
                        code @ (ConnectReturnCode::BadUserNamePassword
                        | ConnectReturnCode::NotAuthorized),
                    )) => {
                        // Read the reason; never retry a refused login.
                        return Ended::Auth(format!("Bambu Cloud refused the sign-in ({code:?})"));
                    }
                    Err(e) => return Ended::Net(e.to_string()),
                },
                _ = shutdown.changed() => {
                    let _ = client.disconnect().await;
                    return Ended::Shutdown;
                }
            }
        }
    }

    /// Bambu answered HTTP 403 to a call on this account. Logged and raised verbatim
    /// (method, path, Bambu's code and message, body), so a refused read and a refused
    /// send can be told apart. Not retried; the token is not marked expired.
    pub async fn refused(&self, account_id: i64, r: &Refusal) {
        warn!(
            account_id,
            method = r.method,
            path = %r.path,
            code = ?r.code,
            reason = ?r.reason,
            body = %r.body,
            "bambu cloud: request refused (HTTP 403)"
        );
        if let Err(e) = bambu_account::mark_error(&self.pool, account_id, &r.to_string()).await {
            warn!(account_id, "bambu cloud: record refusal: {e}");
        }
        let label = bambu_account::get(&self.pool, account_id)
            .await
            .map(|a| a.label)
            .unwrap_or_else(|_| format!("#{account_id}"));
        let kind = refused_attention_kind(account_id);
        let title = format!(
            "Bambu Cloud refused a {} for account \u{201c}{label}\u{201d}: {}",
            r.kind(),
            r.summary()
        );
        let body = format!(
            "{} {} \u{2192} HTTP 403 {}\nResponse body: {}",
            r.method,
            r.path,
            r.summary(),
            r.body
        );
        match crate::repo::attention::raise(&self.pool, None, &kind, &title, Some(&body)).await {
            Ok(raised) => {
                let _ = self.bus_tx.send(ServerEvent::attention_raised(
                    raised.created,
                    AttentionItemEvent {
                        id: raised.item.id,
                        printer_id: None,
                        kind,
                        title: raised.item.title,
                        body: raised.item.body,
                    },
                ));
            }
            Err(e) => warn!(account_id, "bambu cloud: raise refusal attention: {e}"),
        }
    }

    /// A REST call on this account got 401: same as a refused MQTT login. Marks the
    /// token rejected, stops this account's connection and raises one Attention item.
    pub async fn token_rejected(&self, account_id: i64, reason: &str) {
        let label = bambu_account::get(&self.pool, account_id)
            .await
            .map(|a| a.label)
            .unwrap_or_else(|_| format!("#{account_id}"));
        let session = self.session(account_id).await;
        let s = match &session {
            Some(s) => Arc::clone(s),
            None => Arc::new(Session::new(account_id)),
        };
        let _ = s.shutdown.send_replace(true);
        self.auth_failed(&s, &label, reason).await;
    }

    /// The token is dead: record it on this account only, stop, raise one Attention item.
    async fn auth_failed(&self, s: &Session, label: &str, reason: &str) {
        let acct = s.account_id;
        warn!(
            account_id = acct,
            reason, "bambu cloud: token rejected; stopping this account"
        );
        let _ = s.state.send_replace(SessionState::AuthExpired {
            reason: reason.to_owned(),
        });
        if let Err(e) = bambu_account::mark_token_rejected(&self.pool, acct, reason).await {
            warn!(account_id = acct, "bambu cloud: record rejection: {e}");
        }
        let kind = auth_attention_kind(acct);
        let title = format!("Bambu Cloud: re-authenticate account \u{201c}{label}\u{201d}");
        let body = format!(
            "{reason}. Its printers can't be watched over the cloud until you sign in again \
             (Settings \u{2192} Integrations \u{2192} Bambu Cloud)."
        );
        match crate::repo::attention::raise(&self.pool, None, &kind, &title, Some(&body)).await {
            Ok(raised) => {
                let _ = self.bus_tx.send(ServerEvent::attention_raised(
                    raised.created,
                    AttentionItemEvent {
                        id: raised.item.id,
                        printer_id: None,
                        kind,
                        title: raised.item.title,
                        body: raised.item.body,
                    },
                ));
            }
            Err(e) => warn!(account_id = acct, "bambu cloud: raise attention: {e}"),
        }
    }

    async fn clear_auth_attention(&self, account_id: i64) {
        let kind = auth_attention_kind(account_id);
        match crate::repo::attention::clear_open(&self.pool, None, &kind).await {
            Ok(cleared) => {
                if let Some(id) = cleared {
                    let _ = self.bus_tx.send(ServerEvent::AttentionCleared {
                        item_id: id,
                        printer_id: None,
                    });
                }
            }
            Err(e) => warn!(account_id, "bambu cloud: clear attention: {e}"),
        }
    }
}

/// Sleep for the backoff (doubling up to 60 s). False when shut down meanwhile.
async fn backoff_or_stop(backoff: &mut Duration, shutdown: &mut watch::Receiver<bool>) -> bool {
    let wait = *backoff;
    *backoff = (*backoff * 2).min(Duration::from_secs(60));
    tokio::select! {
        _ = tokio::time::sleep(wait) => true,
        _ = shutdown.changed() => false,
    }
}

/// TLS for Bambu's public broker: the platform's root certificates.
fn platform_tls() -> Result<TlsConfiguration, String> {
    use rumqttc::tokio_rustls::rustls::{ClientConfig, RootCertStore};
    let mut roots = RootCertStore::empty();
    let certs = rustls_native_certs::load_native_certs()
        .map_err(|e| format!("load platform certificates: {e}"))?;
    for c in certs {
        // A single unparsable system certificate is skipped, not fatal.
        let _ = roots.add(c);
    }
    if roots.is_empty() {
        return Err("no platform root certificates found".into());
    }
    let cfg = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(TlsConfiguration::Rustls(Arc::new(cfg)))
}

// ---------------------------------------------------------------------------
// Driver: one printer's cloud path
// ---------------------------------------------------------------------------

/// Why commands never reach a printer on its cloud path.
pub const CLOUD_PATH_READ_ONLY: &str =
    "Cloud only: Foreman can watch this printer over Bambu Cloud but can't send to it";

/// A printer's cloud path: reads the account connection's merged reports for its serial.
pub struct BambuCloudDriver {
    printer_id: i64,
    account_id: i64,
    serial: String,
    hub: Arc<CloudHub>,
    bus_tx: Arc<broadcast::Sender<ServerEvent>>,
    status: Arc<RwLock<Option<PrinterStatus>>>,
    caps: Arc<RwLock<PrinterCapabilities>>,
    shutdown_tx: watch::Sender<bool>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl BambuCloudDriver {
    pub fn new(
        printer_id: i64,
        account_id: i64,
        serial: String,
        hub: Arc<CloudHub>,
        bus_tx: Arc<broadcast::Sender<ServerEvent>>,
    ) -> Self {
        let mut caps = PrinterCapabilities::default();
        caps_from_serial(&serial, &mut caps);
        Self {
            printer_id,
            account_id,
            serial,
            hub,
            bus_tx,
            status: Arc::new(RwLock::new(None)),
            caps: Arc::new(RwLock::new(caps)),
            shutdown_tx: watch::channel(false).0,
            task: Mutex::new(None),
        }
    }

    fn spawn_loop(&self, session: Arc<Session>) -> tokio::task::JoinHandle<()> {
        let printer_id = self.printer_id;
        let account_id = self.account_id;
        let serial = self.serial.clone();
        let hub = Arc::clone(&self.hub);
        let bus = Arc::clone(&self.bus_tx);
        let status = Arc::clone(&self.status);
        let caps = Arc::clone(&self.caps);
        let mut shutdown = self.shutdown_tx.subscribe();
        tokio::spawn(async move {
            let mut session = session;
            let mut seq = session.seq.subscribe();
            let mut state_rx = session.state.subscribe();
            let mut online: Option<bool> = None;
            let mut last_reports = u64::MAX;
            let min_interval = Duration::from_millis(500);
            let mut last_publish = Instant::now() - min_interval;
            loop {
                // Follow the account's current connection (a re-authentication
                // replaces it; the printer keeps its path).
                match hub.session(account_id).await {
                    Some(cur) if !Arc::ptr_eq(&cur, &session) => {
                        session = cur;
                        seq = session.seq.subscribe();
                        state_rx = session.state.subscribe();
                        last_reports = u64::MAX;
                    }
                    _ => {}
                }
                // Connection state → this printer's connection events.
                let st = state_rx.borrow_and_update().clone();
                let up = st == SessionState::Connected;
                if online != Some(up) {
                    online = Some(up);
                    let reason = match &st {
                        SessionState::Connected => Some(CLOUD_PATH_NAME.to_owned()),
                        SessionState::AuthExpired { .. } => {
                            Some("Bambu Cloud sign-in expired".to_owned())
                        }
                        SessionState::SignedOut => Some("Bambu Cloud signed out".to_owned()),
                        _ => None,
                    };
                    let kind = if up {
                        ConnectionEventKind::Online
                    } else {
                        ConnectionEventKind::Offline
                    };
                    let _ = bus.send(ServerEvent::PrinterConnection {
                        printer_id,
                        event: ConnectionEvent {
                            kind,
                            active_path: reason,
                        },
                    });
                    if !up && st != SessionState::Connecting {
                        let off = PrinterStatus {
                            state: PrinterState::Offline,
                            ..Default::default()
                        };
                        *status.write().await = Some(off.clone());
                        let _ = bus.send(ServerEvent::PrinterStatus {
                            printer_id,
                            status: off,
                        });
                    }
                }

                // This printer's merged reports → status (only once the state is known).
                if up && last_publish.elapsed() >= min_interval {
                    let snap = {
                        let feeds = session.feeds.read().await;
                        feeds.get(&serial).and_then(|f| {
                            (f.reports != last_reports && f.status.state_known)
                                .then(|| (f.reports, f.status.clone()))
                        })
                    };
                    if let Some((n, cached)) = snap {
                        last_reports = n;
                        last_publish = Instant::now();
                        cached.update_caps(&mut *caps.write().await);
                        let ps = cached.to_printer_status();
                        *status.write().await = Some(ps.clone());
                        let _ = bus.send(ServerEvent::PrinterStatus {
                            printer_id,
                            status: ps,
                        });
                    }
                }

                tokio::select! {
                    r = seq.changed() => if r.is_err() { break },
                    r = state_rx.changed() => if r.is_err() { break },
                    _ = tokio::time::sleep(min_interval) => {},
                    _ = shutdown.changed() => break,
                }
            }
        })
    }
}

#[async_trait]
impl PrinterDriver for BambuCloudDriver {
    async fn connect(&self) -> Result<PrinterCapabilities, AppError> {
        // The account's connection is shared; make sure it exists and covers this serial.
        self.hub.sync().await?;
        let session = self.hub.session(self.account_id).await.ok_or_else(|| {
            AppError::Driver(format!(
                "Bambu account {} has no cloud connection (printer not linked?)",
                self.account_id
            ))
        })?;
        info!(
            printer_id = self.printer_id,
            account_id = self.account_id,
            "bambu cloud: printer on its cloud path"
        );
        *self.task.lock().await = Some(self.spawn_loop(session));
        Ok(self.caps.read().await.clone())
    }

    async fn disconnect(&self) -> Result<(), AppError> {
        // Stops this printer's reader only; the account's connection stays for the others.
        let _ = self.shutdown_tx.send_replace(true);
        if let Some(h) = self.task.lock().await.take() {
            let _ = h.await;
        }
        Ok(())
    }

    async fn probe_capabilities(&self) -> Result<PrinterCapabilities, AppError> {
        Ok(self.caps.read().await.clone())
    }

    async fn send_command(&self, _cmd: PrinterCommand) -> Result<(), AppError> {
        Err(AppError::NotImplemented(CLOUD_PATH_READ_ONLY.into()))
    }

    async fn list_files(&self, _path: Option<&str>) -> Result<Vec<PrinterFile>, AppError> {
        Err(AppError::NotImplemented(CLOUD_PATH_READ_ONLY.into()))
    }

    async fn upload_file(&self, _path: &str, _data: Vec<u8>) -> Result<(), AppError> {
        Err(AppError::NotImplemented(CLOUD_PATH_READ_ONLY.into()))
    }

    async fn delete_file(&self, _path: &str) -> Result<(), AppError> {
        Err(AppError::NotImplemented(CLOUD_PATH_READ_ONLY.into()))
    }

    async fn start_print(&self, _path: &str) -> Result<(), AppError> {
        Err(AppError::NotImplemented(CLOUD_PATH_READ_ONLY.into()))
    }

    async fn current_status(&self) -> Result<PrinterStatus, AppError> {
        Ok(self.status.read().await.clone().unwrap_or(PrinterStatus {
            state: PrinterState::Offline,
            ..Default::default()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One account's token dying marks only that account, raises exactly one Attention
    /// item naming it (however often it is reported), and leaves the other account alone.
    #[tokio::test]
    async fn rejected_token_touches_only_its_account_and_raises_one_item() {
        let dir = tempfile::TempDir::new().expect("dir");
        let db = crate::db::ForemanDb::connect(&dir.path().join("f.db"))
            .await
            .expect("db");
        let pool = db.pool.clone();
        let (tx, mut rx) = broadcast::channel(16);
        let hub = CloudHub::new(
            pool.clone(),
            Arc::new(SecretBox::from_key([7; 32])),
            BambuCloud::new().expect("http"),
            Arc::new(tx),
        );
        let thomas = bambu_account::create(&pool, "Thomas", Region::Global, "t@example.com")
            .await
            .expect("a");
        let akva = bambu_account::create(&pool, "AKVA", Region::Global, "a@example.com")
            .await
            .expect("b");
        for id in [thomas.id, akva.id] {
            bambu_account::set_token(&pool, id, "enc:v1:x", None)
                .await
                .expect("token");
        }

        hub.token_rejected(
            thomas.id,
            "Bambu Cloud refused the sign-in (BadUserNamePassword)",
        )
        .await;
        hub.token_rejected(
            thomas.id,
            "Bambu Cloud refused the sign-in (BadUserNamePassword)",
        )
        .await;

        let t = bambu_account::get(&pool, thomas.id).await.expect("t");
        let a = bambu_account::get(&pool, akva.id).await.expect("a");
        assert!(t.token_rejected_at.is_some());
        assert!(a.token_rejected_at.is_none() && a.last_error.is_none());

        let open = crate::repo::attention::list_active(&pool)
            .await
            .expect("items");
        let auth: Vec<_> = open
            .iter()
            .filter(|i| i.kind.starts_with("bambu_auth:"))
            .collect();
        assert_eq!(auth.len(), 1, "one item, not one per failure");
        assert!(auth[0].title.contains("Thomas"), "{}", auth[0].title);
        assert_eq!(auth[0].kind, auth_attention_kind(thomas.id));
        // Added once, then updated (never added twice).
        let mut added = 0;
        while let Ok(ev) = rx.try_recv() {
            if let ServerEvent::AttentionAdded { .. } = ev {
                added += 1;
            }
        }
        assert_eq!(added, 1);

        // A new token clears the rejection; the item goes with a restart.
        bambu_account::set_token(&pool, thomas.id, "enc:v1:y", None)
            .await
            .expect("token");
        hub.restart(thomas.id).await.expect("restart");
        assert!(bambu_account::get(&pool, thomas.id)
            .await
            .expect("t")
            .token_rejected_at
            .is_none());
        let open = crate::repo::attention::list_active(&pool)
            .await
            .expect("items");
        assert!(!open.iter().any(|i| i.kind.starts_with("bambu_auth:")));
        hub.shutdown().await;
    }

    /// A 403 is raised and logged as Bambu sent it, and does not expire the token.
    #[tokio::test]
    async fn refusal_is_recorded_verbatim_and_does_not_expire_the_token() {
        let dir = tempfile::TempDir::new().expect("dir");
        let db = crate::db::ForemanDb::connect(&dir.path().join("f.db"))
            .await
            .expect("db");
        let pool = db.pool.clone();
        let (tx, _rx) = broadcast::channel(16);
        let hub = CloudHub::new(
            pool.clone(),
            Arc::new(SecretBox::from_key([7; 32])),
            BambuCloud::new().expect("http"),
            Arc::new(tx),
        );
        let a = bambu_account::create(&pool, "AKVA", Region::Global, "a@example.com")
            .await
            .expect("a");
        bambu_account::set_token(&pool, a.id, "enc:v1:x", None)
            .await
            .expect("token");
        let body = r#"{"code":-3120,"error":"internal blocking"}"#;
        let r = Refusal {
            method: "GET",
            path: "/v1/user-service/my/tasks".into(),
            code: Some(-3120),
            reason: Some("internal blocking".into()),
            body: body.into(),
        };
        hub.refused(a.id, &r).await;
        let items = crate::repo::attention::list_active(&pool)
            .await
            .expect("items");
        let item = items
            .iter()
            .find(|i| i.kind == refused_attention_kind(a.id))
            .expect("refusal item");
        assert!(item.title.contains("refused a read"), "{}", item.title);
        assert!(
            item.title.contains("code -3120 \"internal blocking\""),
            "{}",
            item.title
        );
        let item_body = item.body.clone().unwrap_or_default();
        assert!(
            item_body.contains("GET /v1/user-service/my/tasks"),
            "{item_body}"
        );
        assert!(item_body.contains(body), "{item_body}");
        let acct = bambu_account::get(&pool, a.id).await.expect("a");
        assert!(
            acct.token_rejected_at.is_none(),
            "a 403 is not an expired token"
        );
        assert!(acct.last_error.as_deref().is_some_and(|e| e.contains(body)));
    }

    #[test]
    fn cloud_urls_round_trip() {
        assert_eq!(cloud_account_of(&cloud_path_url(7)), Some(7));
        assert_eq!(cloud_account_of("10.0.0.249"), None);
        assert_eq!(cloud_account_of("bambu-cloud://x"), None);
    }

    #[test]
    fn mqtt_username_from_jwt_or_none() {
        use base64::Engine as _;
        let enc = |v: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v);
        let jwt = format!(
            "{}.{}.sig",
            enc("{}"),
            enc(r#"{"preferred_username":"u_123"}"#)
        );
        assert_eq!(
            jwt_mqtt_username(&Token::new(jwt)).as_deref(),
            Some("u_123")
        );
        let bare = format!(
            "{}.{}.sig",
            enc("{}"),
            enc(r#"{"preferred_username":"456"}"#)
        );
        assert_eq!(
            jwt_mqtt_username(&Token::new(bare)).as_deref(),
            Some("u_456")
        );
        // Current Bambu tokens are opaque: the uid comes from /my/profile instead.
        assert_eq!(jwt_mqtt_username(&Token::new("AABBCC".into())), None);
    }

    #[test]
    fn deltas_without_gcode_state_leave_the_state_unknown() {
        let mut c = CachedStatus::default();
        c.merge_report(&serde_json::json!({"bed_temper": 60.0, "nozzle_temper": 210.0}));
        assert!(!c.state_known);
        c.seed_state(&PrinterStatus {
            state: PrinterState::Printing,
            ..Default::default()
        });
        assert!(c.state_known);
        assert_eq!(c.to_printer_status().state, PrinterState::Printing);
        c.merge_report(&serde_json::json!({"gcode_state": "FINISH"}));
        assert_eq!(c.to_printer_status().state, PrinterState::Idle);
    }
}
