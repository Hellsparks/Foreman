//! Bambu Cloud accounts and sign-in (doc 07 §B, Settings → Integrations → Bambu Cloud).
//!
//! - `GET    /api/bambu/accounts` — every account with its state (never the token)
//! - `POST   /api/bambu/accounts` — `{label, region, email}` → new account, signed out
//! - `DELETE /api/bambu/accounts/{id}` — printers that referenced it are unlinked
//! - `POST   /api/bambu/accounts/{id}/signin` — `{password}` → connected | code required
//! - `POST   /api/bambu/accounts/{id}/code` — `{code}` → completes a held sign-in
//! - `POST   /api/bambu/accounts/{id}/token` — `{token}` → validated against /my/profile
//! - `GET    /api/bambu/accounts/{id}/devices` — the account's printers (`/user/bind`),
//!   each with the Foreman printer that has its serial, if any
//! - `POST   /api/bambu/accounts/{id}/devices/{serial}/link` — attach the device to the
//!   printer with that serial, or create one; it gains a Bambu Cloud path
//! - `DELETE /api/bambu/accounts/{id}/devices/{serial}/link` — drop the cloud path
//! - `GET    /api/printers/{id}/cloud-history` — the printer's cloud print history
//!
//! Many accounts by design: every call names one account and only touches that one.
//! A sign-in that needs a second factor is held here, per account, in memory (never the
//! password: the code step does not need it). Nothing retries, nothing re-logs-in in the
//! background. The password, code and token are never logged.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use foreman_core::{
    bambu_cloud::{
        BambuCloud, CloudDevice, CloudError, CloudTask, IssuedToken, LoginOutcome, Password,
        Region, SecondFactor, Token,
    },
    domain::{CreatePrinter, FirmwareKind, Printer},
    error::AppError,
    repo::bambu_account::{self, BambuAccount},
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{info, warn};
use ts_rs::TS;

use super::ssh::{ApiError, ApiResult};
use crate::state::AppState;

/// A held sign-in expires if the code does not arrive in time.
const HOLD_FOR: Duration = Duration::from_secs(15 * 60);
/// "Expires in N days" shows from this many days out.
const WARN_DAYS: i64 = 14;

/// A sign-in waiting for its six-digit code.
#[derive(Debug)]
struct Held {
    region: Region,
    email: String,
    factor: SecondFactor,
    since: Instant,
}

/// Per-account sign-in state. One lock per account: a double-click cannot fire two
/// logins for the same account, and one account's slow call never blocks another's.
#[derive(Debug)]
pub struct BambuSignins {
    cloud: BambuCloud,
    slots: std::sync::Mutex<HashMap<i64, Arc<Mutex<Option<Held>>>>>,
}

impl BambuSignins {
    pub fn new() -> Result<Self, AppError> {
        Ok(Self {
            cloud: BambuCloud::new()?,
            slots: std::sync::Mutex::new(HashMap::new()),
        })
    }

    /// Use a given cloud client (a mock Bambu in tests).
    pub fn with_cloud(cloud: BambuCloud) -> Self {
        Self {
            cloud,
            slots: std::sync::Mutex::new(HashMap::new()),
        }
    }

    fn slot(&self, id: i64) -> Arc<Mutex<Option<Held>>> {
        let mut map = self.slots.lock().unwrap_or_else(|p| p.into_inner());
        Arc::clone(map.entry(id).or_default())
    }

    fn forget(&self, id: i64) {
        self.slots
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&id);
    }

    /// Which factor a held sign-in waits for, if any (expired holds read as none).
    async fn pending(&self, id: i64) -> Option<CodeChannel> {
        let slot = self.slot(id);
        let held = slot.lock().await;
        held.as_ref()
            .filter(|h| h.since.elapsed() < HOLD_FOR)
            .map(|h| CodeChannel::of(&h.factor))
    }
}

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

/// Where the six-digit code comes from. The UI shows one form for both.
#[derive(Debug, Clone, Copy, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CodeChannel {
    Email,
    Authenticator,
}

impl CodeChannel {
    fn of(f: &SecondFactor) -> Self {
        match f {
            SecondFactor::Email => Self::Email,
            SecondFactor::Totp { .. } => Self::Authenticator,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AccountState {
    SignedOut,
    CodeRequired,
    Connected,
    Expiring,
    Expired,
    Error,
}

/// An account card. Never carries the token.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct BambuAccountView {
    #[serde(flatten)]
    pub account: BambuAccount,
    pub state: AccountState,
    /// Whole days until the token expires (when known and not yet expired).
    #[ts(type = "number | null")]
    pub days_left: Option<i64>,
    /// A held sign-in is waiting for a code from here.
    pub code_channel: Option<CodeChannel>,
}

fn view(
    account: BambuAccount,
    code_channel: Option<CodeChannel>,
    now: DateTime<Utc>,
) -> BambuAccountView {
    let days_left = account
        .token_expires_at
        .filter(|e| *e > now)
        .map(|e| (e - now).num_days());
    let expired = account.has_token && account.token_expires_at.is_some_and(|e| e <= now);
    let state = if !account.has_token {
        match (code_channel, &account.last_error) {
            (Some(_), _) => AccountState::CodeRequired,
            (None, Some(_)) => AccountState::Error,
            (None, None) => AccountState::SignedOut,
        }
    } else if expired || account.token_rejected_at.is_some() {
        AccountState::Expired
    } else if account.last_error.is_some() {
        AccountState::Error
    } else if days_left.is_some_and(|d| d <= WARN_DAYS) {
        AccountState::Expiring
    } else {
        AccountState::Connected
    };
    BambuAccountView {
        account,
        state,
        days_left,
        code_channel,
    }
}

async fn load_view(s: &AppState, id: i64) -> ApiResult<BambuAccountView> {
    let a = bambu_account::get(&s.foreman_db.pool, id).await?;
    let ch = s.bambu.pending(id).await;
    Ok(view(a, ch, Utc::now()))
}

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct CreateBambuAccount {
    pub label: String,
    pub region: Region,
    pub email: String,
}

#[derive(Deserialize)]
pub struct SigninBody {
    password: Password,
}

#[derive(Deserialize)]
pub struct CodeBody {
    code: String,
}

#[derive(Deserialize)]
pub struct TokenBody {
    token: Token,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct DeviceList {
    pub devices: Vec<DeviceRow>,
}

/// A device on the account card, with the Foreman printer that has its serial.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct DeviceRow {
    #[serde(flatten)]
    #[ts(flatten)]
    pub device: CloudDevice,
    /// The Foreman printer with this serial (attach), or None (create).
    #[ts(type = "number | null")]
    pub printer_id: Option<i64>,
    pub printer_name: Option<String>,
    /// The printer already has this account's cloud path.
    pub linked: bool,
    /// What this account's cloud connection has heard from the device (linked only).
    pub feed: Option<foreman_core::drivers::bambu_cloud::FeedInfo>,
}

/// Result of linking a device.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct LinkResult {
    #[ts(type = "number")]
    pub printer_id: i64,
    /// A new printer was created for the device (no printer had its serial).
    pub created: bool,
}

/// The printer's Bambu Cloud print history.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct CloudHistory {
    pub account_label: String,
    pub tasks: Vec<CloudTask>,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

fn cloud_err(e: &CloudError) -> ApiError {
    let status = match e {
        CloudError::Rejected(_) | CloudError::Unauthorized(_) => StatusCode::UNPROCESSABLE_ENTITY,
        CloudError::Unreachable(_) | CloudError::Forbidden(_) => StatusCode::BAD_GATEWAY,
    };
    ApiError::new(status, e.to_string())
}

/// Record a sign-in failure on this account only. An account that still holds a token
/// keeps its state: a mistyped re-authentication does not break a working token.
async fn record_signin_failure(s: &AppState, id: i64, e: &CloudError) -> ApiError {
    let pool = &s.foreman_db.pool;
    if let CloudError::Forbidden(r) = e {
        if let Some(hub) = s.driver_manager.cloud().await {
            hub.refused(id, r).await;
        }
    }
    let has_token = bambu_account::get_sealed_token(pool, id)
        .await
        .ok()
        .flatten()
        .is_some();
    if !has_token {
        if let Err(db) = bambu_account::mark_error(pool, id, &e.to_string()).await {
            warn!(account_id = id, error = %db, "bambu: could not record sign-in error");
        }
    }
    cloud_err(e)
}

async fn store_token(s: &AppState, id: i64, t: &IssuedToken) -> ApiResult<()> {
    let sealed = s.secrets.seal(t.token.expose())?;
    bambu_account::set_token(&s.foreman_db.pool, id, &sealed, t.expires_at).await?;
    info!(account_id = id, expires_at = ?t.expires_at, "bambu: token stored");
    // A new token ends an expired state: reconnect this account (only this one).
    if let Some(hub) = s.driver_manager.cloud().await {
        if let Err(e) = hub.restart(id).await {
            warn!(account_id = id, "bambu cloud: restart after new token: {e}");
        }
    }
    Ok(())
}

/// The account's region and token, or 409 when it is signed out.
async fn account_token(s: &AppState, id: i64) -> ApiResult<(BambuAccount, Token)> {
    let pool = &s.foreman_db.pool;
    let a = bambu_account::get(pool, id).await?;
    let sealed = bambu_account::get_sealed_token(pool, id)
        .await?
        .ok_or_else(|| ApiError::new(StatusCode::CONFLICT, "this account is not signed in"))?;
    Ok((a, Token::new(s.secrets.open(&sealed)?)))
}

/// A REST call on this account failed. A rejected token (401) stops the account's cloud
/// connection and raises its Attention item (once); a refusal (403) is logged and raised
/// verbatim, not retried; other errors are only recorded.
async fn record_call_failure(s: &AppState, id: i64, e: &CloudError) -> ApiError {
    match e {
        CloudError::Forbidden(r) => match s.driver_manager.cloud().await {
            Some(hub) => hub.refused(id, r).await,
            None => {
                warn!(account_id = id, refusal = %r, "bambu: request refused (HTTP 403)");
                if let Err(db) =
                    bambu_account::mark_error(&s.foreman_db.pool, id, &r.to_string()).await
                {
                    warn!(account_id = id, error = %db, "bambu: could not record refusal");
                }
            }
        },
        CloudError::Unauthorized(_) => match s.driver_manager.cloud().await {
            Some(hub) => hub.token_rejected(id, &e.to_string()).await,
            None => {
                if let Err(db) =
                    bambu_account::mark_token_rejected(&s.foreman_db.pool, id, &e.to_string()).await
                {
                    warn!(account_id = id, error = %db, "bambu: could not record rejection");
                }
            }
        },
        _ => {
            if let Err(db) = bambu_account::mark_error(&s.foreman_db.pool, id, &e.to_string()).await
            {
                warn!(account_id = id, error = %db, "bambu: could not record error");
            }
        }
    }
    cloud_err(e)
}

/// Bambu printers by serial.
async fn printers_by_serial(s: &AppState) -> ApiResult<Vec<Printer>> {
    Ok(foreman_core::repo::printer::list(&s.foreman_db.pool)
        .await?
        .into_iter()
        .filter(|p| p.firmware == FirmwareKind::Bambu)
        .collect())
}

/// Restart a printer's driver after its cloud link changed (not with drivers off).
async fn relink(s: &AppState, printer_id: i64) -> Result<(), AppError> {
    if !s.drivers_enabled {
        return Ok(());
    }
    s.driver_manager.cloud_link_changed(printer_id).await
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

pub async fn list(State(s): State<AppState>) -> ApiResult<Json<Vec<BambuAccountView>>> {
    let now = Utc::now();
    let mut out = Vec::new();
    for a in bambu_account::list(&s.foreman_db.pool).await? {
        let ch = s.bambu.pending(a.id).await;
        out.push(view(a, ch, now));
    }
    Ok(Json(out))
}

pub async fn create(
    State(s): State<AppState>,
    Json(req): Json<CreateBambuAccount>,
) -> ApiResult<(StatusCode, Json<BambuAccountView>)> {
    let a = bambu_account::create(&s.foreman_db.pool, &req.label, req.region, &req.email).await?;
    info!(
        account_id = a.id,
        region = a.region.as_str(),
        "bambu: account added"
    );
    Ok((StatusCode::CREATED, Json(view(a, None, Utc::now()))))
}

pub async fn remove(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let linked: Vec<i64> = printers_by_serial(&s)
        .await?
        .into_iter()
        .filter(|p| p.bambu_account_id == Some(id))
        .map(|p| p.id)
        .collect();
    bambu_account::delete(&s.foreman_db.pool, id).await?;
    s.bambu.forget(id);
    info!(account_id = id, "bambu: account removed");
    // Its printers lose their cloud path (FK SET NULL); the connection goes with it.
    for pid in linked {
        if let Err(e) = relink(&s, pid).await {
            warn!(
                printer_id = pid,
                "bambu: drop cloud path after account removal: {e}"
            );
        }
    }
    if let Some(hub) = s.driver_manager.cloud().await {
        hub.sync().await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn signin(
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<SigninBody>,
) -> ApiResult<Json<BambuAccountView>> {
    if body.password.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "password is required",
        ));
    }
    let a = bambu_account::get(&s.foreman_db.pool, id).await?;
    let slot = s.bambu.slot(id);
    let mut held = slot.lock().await;
    *held = None;
    info!(
        account_id = id,
        region = a.region.as_str(),
        "bambu: sign-in with password"
    );
    match s
        .bambu
        .cloud
        .login_password(a.region, &a.email, &body.password)
        .await
    {
        Ok(LoginOutcome::Token(t)) => {
            drop(held);
            store_token(&s, id, &t).await?;
        }
        Ok(LoginOutcome::NeedsCode(factor)) => {
            info!(account_id = id, channel = ?CodeChannel::of(&factor), "bambu: second factor required");
            *held = Some(Held {
                region: a.region,
                email: a.email,
                factor,
                since: Instant::now(),
            });
            drop(held);
        }
        Err(e) => {
            drop(held);
            warn!(account_id = id, error = %e, "bambu: sign-in failed");
            return Err(record_signin_failure(&s, id, &e).await);
        }
    }
    Ok(Json(load_view(&s, id).await?))
}

pub async fn code(
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<CodeBody>,
) -> ApiResult<Json<BambuAccountView>> {
    let code = body.code.trim();
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "the code is six digits",
        ));
    }
    let slot = s.bambu.slot(id);
    let mut held = slot.lock().await;
    let Some(h) = held.as_ref().filter(|h| h.since.elapsed() < HOLD_FOR) else {
        *held = None;
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "no sign-in is waiting for a code (it may have timed out); sign in again",
        ));
    };
    match s
        .bambu
        .cloud
        .submit_code(h.region, &h.email, &h.factor, code)
        .await
    {
        Ok(t) => {
            *held = None;
            drop(held);
            store_token(&s, id, &t).await?;
            Ok(Json(load_view(&s, id).await?))
        }
        Err(e) => {
            // Keep the hold: the user may retype the code. Nothing is retried for them.
            drop(held);
            warn!(account_id = id, error = %e, "bambu: code rejected");
            Err(cloud_err(&e))
        }
    }
}

pub async fn paste_token(
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<TokenBody>,
) -> ApiResult<Json<BambuAccountView>> {
    if body.token.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "token is required"));
    }
    let a = bambu_account::get(&s.foreman_db.pool, id).await?;
    let slot = s.bambu.slot(id);
    let mut held = slot.lock().await;
    info!(account_id = id, "bambu: validating pasted token");
    match s.bambu.cloud.profile(a.region, &body.token).await {
        Ok(p) => {
            *held = None;
            drop(held);
            info!(account_id = id, uid = ?p.uid, "bambu: pasted token accepted");
            let expires_at = foreman_core::bambu_cloud::jwt_expiry(&body.token);
            store_token(
                &s,
                id,
                &IssuedToken {
                    token: body.token,
                    expires_at,
                },
            )
            .await?;
            Ok(Json(load_view(&s, id).await?))
        }
        Err(e) => {
            drop(held);
            warn!(account_id = id, error = %e, "bambu: pasted token rejected");
            Err(record_signin_failure(&s, id, &e).await)
        }
    }
}

pub async fn devices(
    State(s): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<DeviceList>> {
    let pool = &s.foreman_db.pool;
    let (a, token) = account_token(&s, id).await?;
    match s.bambu.cloud.devices(a.region, &token).await {
        Ok(devices) => {
            bambu_account::mark_ok(pool, id).await?;
            let printers = printers_by_serial(&s).await?;
            let hub = s.driver_manager.cloud().await;
            let mut rows = Vec::new();
            for d in devices {
                let p = printers
                    .iter()
                    .find(|p| p.serial_number.as_deref() == Some(d.dev_id.as_str()));
                let feed = match &hub {
                    Some(h) => h.feed_info(id, &d.dev_id).await,
                    None => None,
                };
                rows.push(DeviceRow {
                    printer_id: p.map(|p| p.id),
                    printer_name: p.map(|p| p.name.clone()),
                    linked: p.is_some_and(|p| p.bambu_account_id == Some(id)),
                    feed,
                    device: d,
                });
            }
            let devices = rows;
            Ok(Json(DeviceList { devices }))
        }
        Err(e) => {
            warn!(account_id = id, error = %e, "bambu: device list failed");
            Err(record_call_failure(&s, id, &e).await)
        }
    }
}

/// Give the printer with this serial a Bambu Cloud path on this account, or create a
/// Bambu printer for the device when none has its serial.
pub async fn link(
    State(s): State<AppState>,
    Path((id, serial)): Path<(i64, String)>,
) -> ApiResult<Json<LinkResult>> {
    let pool = &s.foreman_db.pool;
    let existing = printers_by_serial(&s)
        .await?
        .into_iter()
        .find(|p| p.serial_number.as_deref() == Some(serial.as_str()));
    let (a, token) = account_token(&s, id).await?;
    let (printer_id, created) = match existing {
        Some(p) => {
            foreman_core::repo::printer::set_bambu_account(pool, p.id, Some(id)).await?;
            info!(
                account_id = id,
                printer_id = p.id,
                "bambu: device attached to printer"
            );
            (p.id, false)
        }
        None => {
            // The device must be on this account; its name and LAN access code come
            // from Bambu (the LAN address does not: add a path to use LAN).
            let device = match s.bambu.cloud.devices(a.region, &token).await {
                Ok(list) => list.into_iter().find(|d| d.dev_id == serial),
                Err(e) => return Err(record_call_failure(&s, id, &e).await),
            }
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::NOT_FOUND,
                    format!(
                        "{serial} is not a printer on account \u{201c}{}\u{201d}",
                        a.label
                    ),
                )
            })?;
            let access_code = match s.bambu.cloud.access_codes(a.region, &token).await {
                Ok(codes) => codes.get(&serial).cloned(),
                Err(e) => {
                    warn!(account_id = id, error = %e, "bambu: access codes unavailable");
                    None
                }
            };
            let name = if device.name.trim().is_empty() {
                serial.clone()
            } else {
                device.name.clone()
            };
            let p = foreman_core::repo::printer::create(
                pool,
                CreatePrinter {
                    name,
                    firmware: FirmwareKind::Bambu,
                    primary_url: None,
                    api_key: None,
                    serial_number: Some(serial.clone()),
                    access_code,
                    tags: vec![],
                    hardened_nozzle: false,
                    slicer_profiles: vec![],
                    sort_order: None,
                    fail_after_checks: None,
                    return_after_secs: None,
                },
            )
            .await?;
            foreman_core::repo::printer::set_bambu_account(pool, p.id, Some(id)).await?;
            info!(
                account_id = id,
                printer_id = p.id,
                "bambu: printer created from device"
            );
            (p.id, true)
        }
    };
    relink(&s, printer_id).await?;
    Ok(Json(LinkResult {
        printer_id,
        created,
    }))
}

/// Remove the printer's Bambu Cloud path (the printer stays).
pub async fn unlink(
    State(s): State<AppState>,
    Path((id, serial)): Path<(i64, String)>,
) -> ApiResult<StatusCode> {
    let p = printers_by_serial(&s)
        .await?
        .into_iter()
        .find(|p| {
            p.serial_number.as_deref() == Some(serial.as_str()) && p.bambu_account_id == Some(id)
        })
        .ok_or_else(|| {
            ApiError::new(StatusCode::NOT_FOUND, "no printer is linked to that device")
        })?;
    foreman_core::repo::printer::set_bambu_account(&s.foreman_db.pool, p.id, None).await?;
    info!(
        account_id = id,
        printer_id = p.id,
        "bambu: cloud path removed"
    );
    relink(&s, p.id).await?;
    if let Some(hub) = s.driver_manager.cloud().await {
        hub.sync().await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/printers/{id}/cloud-history` — read-only cloud print history (20 newest).
pub async fn printer_history(
    State(s): State<AppState>,
    Path(printer_id): Path<i64>,
) -> ApiResult<Json<CloudHistory>> {
    let p = foreman_core::repo::printer::get(&s.foreman_db.pool, printer_id).await?;
    let (Some(id), Some(serial)) = (p.bambu_account_id, p.serial_number.clone()) else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "this printer is not linked to a Bambu Cloud account",
        ));
    };
    let (a, token) = account_token(&s, id).await?;
    if a.token_rejected_at.is_some() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!(
                "re-authenticate Bambu account \u{201c}{}\u{201d} to see its history",
                a.label
            ),
        ));
    }
    match s.bambu.cloud.tasks(a.region, &token, &serial, 20).await {
        Ok(tasks) => Ok(Json(CloudHistory {
            account_label: a.label,
            tasks,
        })),
        Err(e) => {
            warn!(account_id = id, printer_id, error = %e, "bambu: history failed");
            Err(record_call_failure(&s, id, &e).await)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foreman_core::bambu_cloud::Region;

    fn acct(has_token: bool, exp_days: Option<i64>, err: Option<&str>) -> BambuAccount {
        let now = Utc::now();
        BambuAccount {
            id: 1,
            label: "x".into(),
            region: Region::Global,
            email: "x@example.com".into(),
            has_token,
            token_expires_at: exp_days
                .map(|d| now + chrono::Duration::days(d) + chrono::Duration::hours(1)),
            last_ok_at: None,
            last_error: err.map(str::to_owned),
            token_rejected_at: None,
            printer_count: 0,
            created_at: now,
        }
    }

    fn st(a: BambuAccount, ch: Option<CodeChannel>) -> String {
        format!("{:?}", view(a, ch, Utc::now()).state)
    }

    #[test]
    fn states() {
        assert_eq!(st(acct(false, None, None), None), "SignedOut");
        assert_eq!(
            st(acct(false, None, None), Some(CodeChannel::Email)),
            "CodeRequired"
        );
        assert_eq!(st(acct(false, None, Some("bad")), None), "Error");
        assert_eq!(st(acct(true, Some(80), None), None), "Connected");
        assert_eq!(st(acct(true, None, None), None), "Connected");
        assert_eq!(st(acct(true, Some(5), None), None), "Expiring");
        assert_eq!(st(acct(true, Some(-2), None), None), "Expired");
        assert_eq!(st(acct(true, Some(80), Some("401")), None), "Error");
        assert_eq!(
            view(acct(true, Some(5), None), None, Utc::now()).days_left,
            Some(5)
        );
    }
}
