//! Bambu Cloud accounts and sign-in (doc 07 §B, Settings → Integrations → Bambu Cloud).
//!
//! - `GET    /api/bambu/accounts` — every account with its state (never the token)
//! - `POST   /api/bambu/accounts` — `{label, region, email}` → new account, signed out
//! - `DELETE /api/bambu/accounts/{id}` — printers that referenced it are unlinked
//! - `POST   /api/bambu/accounts/{id}/signin` — `{password}` → connected | code required
//! - `POST   /api/bambu/accounts/{id}/code` — `{code}` → completes a held sign-in
//! - `POST   /api/bambu/accounts/{id}/token` — `{token}` → validated against /my/profile
//! - `GET    /api/bambu/accounts/{id}/devices` — the account's printers (`/user/bind`)
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
        BambuCloud, CloudDevice, CloudError, IssuedToken, LoginOutcome, Password, Region,
        SecondFactor, Token,
    },
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
    } else if expired {
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
    pub devices: Vec<CloudDevice>,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

fn cloud_err(e: &CloudError) -> ApiError {
    let status = match e {
        CloudError::Rejected(_) | CloudError::Unauthorized(_) => StatusCode::UNPROCESSABLE_ENTITY,
        CloudError::Unreachable(_) => StatusCode::BAD_GATEWAY,
    };
    ApiError::new(status, e.to_string())
}

/// Record a sign-in failure on this account only. An account that still holds a token
/// keeps its state: a mistyped re-authentication does not break a working token.
async fn record_signin_failure(s: &AppState, id: i64, e: &CloudError) -> ApiError {
    let pool = &s.foreman_db.pool;
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
    Ok(())
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
    bambu_account::delete(&s.foreman_db.pool, id).await?;
    s.bambu.forget(id);
    info!(account_id = id, "bambu: account removed");
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
    let a = bambu_account::get(pool, id).await?;
    let sealed = bambu_account::get_sealed_token(pool, id)
        .await?
        .ok_or_else(|| ApiError::new(StatusCode::CONFLICT, "this account is not signed in"))?;
    let token = Token::new(s.secrets.open(&sealed)?);
    match s.bambu.cloud.devices(a.region, &token).await {
        Ok(devices) => {
            bambu_account::mark_ok(pool, id).await?;
            Ok(Json(DeviceList { devices }))
        }
        Err(e) => {
            warn!(account_id = id, error = %e, "bambu: device list failed");
            bambu_account::mark_error(pool, id, &e.to_string()).await?;
            Err(cloud_err(&e))
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
