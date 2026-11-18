//! Bambu Lab cloud HTTP client: sign-in and the device list (doc 07 §B).
//!
//! Stateless: every call takes the region and (where needed) the token, so nothing here
//! knows about "the" account. The caller owns per-account state.
//!
//! Rules: the password is only ever moved into the login request body. It is wrapped in
//! [`Password`] (redacted `Debug`, no `Display`) and never logged. Tokens are wrapped in
//! [`Token`] for the same reason. Nothing here retries.
//!
//! Field names come from (read, not copied):
//! - m1guelpf/bambulab-rs `src/lib.rs`, `src/types.rs` (MIT): `accessToken`, bind `devices[]`
//!   (`dev_id`, `name`, `online`, `print_status`, `dev_model_name`, `dev_product_name`,
//!   `nozzle_diameter`), profile (`uid`, `account`, `name`).
//! - Doridian/OpenBambuAPI `cloud-http.md`: `loginType` (`""` = token issued,
//!   `verifyCode` = email code), `expiresIn` (seconds), login with `{account, code}`,
//!   error shape `{message, code, error}`.
//! - coelacant1/Bambu-Lab-Cloud-API `API_AUTHENTICATION.md`: `loginType: "tfa"` +
//!   `tfaKey`, completed with `POST /api/sign-in/tfa {tfaKey, tfaCode}`; email code requested
//!   with `POST /v1/user-service/user/sendemail/code {email, type: "codeLogin"}`.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::error::AppError;

/// Global (`api.bambulab.com`) or China (`api.bambulab.cn`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Region {
    Global,
    China,
}

impl Region {
    pub fn parse(s: &str) -> Result<Self, AppError> {
        match s {
            "global" => Ok(Self::Global),
            "china" => Ok(Self::China),
            other => Err(AppError::InvalidInput(format!(
                "region must be 'global' or 'china', got '{other}'"
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::China => "china",
        }
    }

    /// Base of the REST API (`/v1/...`).
    pub fn api_base(self) -> &'static str {
        match self {
            Self::Global => "https://api.bambulab.com",
            Self::China => "https://api.bambulab.cn",
        }
    }

    /// Base of the website, which serves the authenticator-app (TOTP) step.
    pub fn web_base(self) -> &'static str {
        match self {
            Self::Global => "https://bambulab.com",
            Self::China => "https://bambulab.cn",
        }
    }
}

/// A password in flight. Redacted `Debug`, no `Display`, never stored.
#[derive(Deserialize)]
#[serde(transparent)]
pub struct Password(String);

impl Password {
    pub fn new(s: String) -> Self {
        Self(s)
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for Password {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Password(<redacted>)")
    }
}

/// An access token (trimmed: pasted tokens often carry a newline). Redacted `Debug`,
/// no `Display`.
#[derive(Clone)]
pub struct Token(String);

impl<'de> Deserialize<'de> for Token {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d).map(Token::new)
    }
}

impl Token {
    pub fn new(s: String) -> Self {
        Self(s.trim().to_owned())
    }
    /// The raw value, for sealing into the DB or an `Authorization` header only.
    pub fn expose(&self) -> &str {
        &self.0
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

/// A token and when it stops working (`None` = Bambu did not say).
#[derive(Debug, Clone)]
pub struct IssuedToken {
    pub token: Token,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Which second factor the account uses. One code form serves both.
#[derive(Debug, Clone)]
pub enum SecondFactor {
    /// A code was emailed; complete with `POST login {account, code}`.
    Email,
    /// Authenticator app; complete with `POST <web>/api/sign-in/tfa {tfaKey, tfaCode}`.
    Totp { tfa_key: String },
}

#[derive(Debug)]
pub enum LoginOutcome {
    Token(IssuedToken),
    NeedsCode(SecondFactor),
}

/// Why a Bambu call failed. `Rejected` = Bambu said no (wrong password/code, expired
/// token); `Unreachable` = network/Cloudflare/unexpected response.
#[derive(Debug, thiserror::Error)]
pub enum CloudError {
    #[error("{0}")]
    Rejected(String),
    #[error("token rejected by Bambu (HTTP {0}); re-authenticate")]
    Unauthorized(u16),
    #[error("{0}")]
    Unreachable(String),
}

/// One printer from `GET /v1/iot-service/api/user/bind`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CloudDevice {
    /// Serial number.
    pub dev_id: String,
    pub name: String,
    pub online: bool,
    /// Bambu's internal model code (e.g. `N2S` for an A1).
    pub dev_model_name: Option<String>,
    /// Human model name (e.g. `A1`).
    pub dev_product_name: Option<String>,
    pub print_status: Option<String>,
    pub nozzle_diameter: Option<f64>,
}

/// The parts of `GET /v1/user-service/my/profile` we use.
#[derive(Debug, Clone)]
pub struct Profile {
    pub uid: Option<i64>,
    pub account: Option<String>,
    pub name: Option<String>,
}

/// Bambu cloud client. Cheap to clone; holds only an HTTP client.
#[derive(Debug, Clone)]
pub struct BambuCloud {
    http: reqwest::Client,
    /// Tests only: (api, web) bases used instead of the region's.
    bases: Option<(String, String)>,
}

const TIMEOUT: Duration = Duration::from_secs(20);

impl BambuCloud {
    pub fn new() -> Result<Self, AppError> {
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(concat!("Foreman/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| AppError::Internal(format!("http client: {e}")))?;
        Ok(Self { http, bases: None })
    }

    /// Point every region at `api` / `web` (a mock server in tests).
    pub fn with_bases(mut self, api: String, web: String) -> Self {
        self.bases = Some((api, web));
        self
    }

    fn api(&self, region: Region) -> &str {
        self.bases
            .as_ref()
            .map_or(region.api_base(), |b| b.0.as_str())
    }

    fn web(&self, region: Region) -> &str {
        self.bases
            .as_ref()
            .map_or(region.web_base(), |b| b.1.as_str())
    }

    /// Step 1: email + password. Returns a token, or which second factor is needed (and,
    /// for email, asks Bambu to send the code). Never retries.
    pub async fn login_password(
        &self,
        region: Region,
        email: &str,
        password: &Password,
    ) -> Result<LoginOutcome, CloudError> {
        let url = format!("{}/v1/user-service/user/login", self.api(region));
        let body = serde_json::json!({ "account": email, "password": password.0 });
        let (status, _headers, v) = self.post_json(&url, &body, None).await?;
        let login_type = v.get("loginType").and_then(Value::as_str).unwrap_or("");
        if let Some(tok) = access_token(&v) {
            return Ok(LoginOutcome::Token(issued(tok, &v)));
        }
        match login_type {
            "verifyCode" => {
                self.send_email_code(region, email).await?;
                Ok(LoginOutcome::NeedsCode(SecondFactor::Email))
            }
            "tfa" => {
                let tfa_key = v
                    .get("tfaKey")
                    .and_then(Value::as_str)
                    .filter(|k| !k.is_empty())
                    .ok_or_else(|| {
                        CloudError::Unreachable(
                            "Bambu asked for an authenticator code but sent no tfaKey".into(),
                        )
                    })?;
                Ok(LoginOutcome::NeedsCode(SecondFactor::Totp {
                    tfa_key: tfa_key.to_owned(),
                }))
            }
            _ => Err(rejection(status, &v, "sign-in failed")),
        }
    }

    async fn send_email_code(&self, region: Region, email: &str) -> Result<(), CloudError> {
        let url = format!("{}/v1/user-service/user/sendemail/code", self.api(region));
        let body = serde_json::json!({ "email": email, "type": "codeLogin" });
        let (status, _h, v) = self.post_json(&url, &body, None).await?;
        if status.is_success() && !is_error_body(&v) {
            Ok(())
        } else {
            Err(rejection(status, &v, "Bambu could not send the email code"))
        }
    }

    /// Step 2: the six-digit code, routed by which factor step 1 reported.
    pub async fn submit_code(
        &self,
        region: Region,
        email: &str,
        factor: &SecondFactor,
        code: &str,
    ) -> Result<IssuedToken, CloudError> {
        match factor {
            SecondFactor::Email => {
                let url = format!("{}/v1/user-service/user/login", self.api(region));
                let body = serde_json::json!({ "account": email, "code": code });
                let (status, _h, v) = self.post_json(&url, &body, None).await?;
                access_token(&v)
                    .map(|t| issued(t, &v))
                    .ok_or_else(|| rejection(status, &v, "code not accepted"))
            }
            SecondFactor::Totp { tfa_key } => {
                let url = format!("{}/api/sign-in/tfa", self.web(region));
                let body = serde_json::json!({ "tfaKey": tfa_key, "tfaCode": code });
                let (status, headers, v) = self.post_json(&url, &body, None).await?;
                // The website answers with the token as a `token` cookie; accept a JSON
                // `accessToken` too.
                if let Some(t) = access_token(&v) {
                    return Ok(issued(t, &v));
                }
                if let Some(t) = cookie(&headers, "token") {
                    return Ok(issued(t, &v));
                }
                Err(rejection(status, &v, "code not accepted"))
            }
        }
    }

    /// `GET /v1/user-service/my/profile` — proves a pasted token works.
    pub async fn profile(&self, region: Region, token: &Token) -> Result<Profile, CloudError> {
        let url = format!("{}/v1/user-service/my/profile", self.api(region));
        let v = self.get_json(&url, token).await?;
        Ok(Profile {
            uid: v.get("uid").and_then(Value::as_i64),
            account: v.get("account").and_then(Value::as_str).map(str::to_owned),
            name: v.get("name").and_then(Value::as_str).map(str::to_owned),
        })
    }

    /// `GET /v1/iot-service/api/user/bind` — the printers bound to this account.
    pub async fn devices(
        &self,
        region: Region,
        token: &Token,
    ) -> Result<Vec<CloudDevice>, CloudError> {
        let url = format!("{}/v1/iot-service/api/user/bind", self.api(region));
        let v = self.get_json(&url, token).await?;
        let list = v
            .get("devices")
            .and_then(Value::as_array)
            .ok_or_else(|| CloudError::Unreachable("device list response had no devices".into()))?;
        Ok(list.iter().filter_map(parse_device).collect())
    }

    async fn get_json(&self, url: &str, token: &Token) -> Result<Value, CloudError> {
        let resp = self
            .http
            .get(url)
            .bearer_auth(token.expose())
            .send()
            .await
            .map_err(net_err)?;
        let status = resp.status();
        let text = resp.text().await.map_err(net_err)?;
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            // A Cloudflare block is also 403 but HTML; tell the two apart.
            if serde_json::from_str::<Value>(&text).is_ok() || text.trim().is_empty() {
                return Err(CloudError::Unauthorized(status.as_u16()));
            }
        }
        let v: Value = serde_json::from_str(&text).map_err(|_| unexpected(status, &text))?;
        if !status.is_success() || is_error_body(&v) {
            return Err(rejection(status, &v, "request failed"));
        }
        Ok(v)
    }

    async fn post_json(
        &self,
        url: &str,
        body: &Value,
        token: Option<&Token>,
    ) -> Result<(reqwest::StatusCode, reqwest::header::HeaderMap, Value), CloudError> {
        let mut req = self.http.post(url).json(body);
        if let Some(t) = token {
            req = req.bearer_auth(t.expose());
        }
        let resp = req.send().await.map_err(net_err)?;
        let status = resp.status();
        let headers = resp.headers().clone();
        let text = resp.text().await.map_err(net_err)?;
        let v = if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).map_err(|_| unexpected(status, &text))?
        };
        Ok((status, headers, v))
    }
}

fn net_err(e: reqwest::Error) -> CloudError {
    // reqwest errors carry the URL, never the body.
    CloudError::Unreachable(format!("could not reach Bambu: {e}"))
}

/// Non-JSON answer (Cloudflare challenge page, proxy error). Only a short, single-line
/// prefix is kept; these bodies never contain our request.
fn unexpected(status: reqwest::StatusCode, text: &str) -> CloudError {
    let snippet: String = text.chars().filter(|c| !c.is_control()).take(120).collect();
    CloudError::Unreachable(format!(
        "unexpected response from Bambu (HTTP {}): {snippet}",
        status.as_u16()
    ))
}

fn access_token(v: &Value) -> Option<String> {
    v.get("accessToken")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

fn issued(token: String, v: &Value) -> IssuedToken {
    let from_body = v
        .get("expiresIn")
        .and_then(Value::as_i64)
        .filter(|s| *s > 0)
        .map(|s| Utc::now() + chrono::Duration::seconds(s));
    let token = Token::new(token);
    let expires_at = from_body.or_else(|| jwt_expiry(&token));
    IssuedToken { token, expires_at }
}

/// `exp` from a JWT payload, if the token is one (not verified; only for display).
pub fn jwt_expiry(token: &Token) -> Option<DateTime<Utc>> {
    use base64::Engine as _;
    let payload = token.expose().split('.').nth(1)?;
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let v: Value = serde_json::from_slice(&raw).ok()?;
    DateTime::from_timestamp(v.get("exp")?.as_i64()?, 0)
}

/// Bambu error bodies carry a non-null `code` and/or an `error` string.
fn is_error_body(v: &Value) -> bool {
    let code_bad = v
        .get("code")
        .is_some_and(|c| !c.is_null() && c.as_i64() != Some(0));
    let err = v
        .get("error")
        .and_then(Value::as_str)
        .is_some_and(|e| !e.is_empty());
    code_bad || err
}

fn rejection(status: reqwest::StatusCode, v: &Value, fallback: &str) -> CloudError {
    let msg = ["message", "error", "msg"]
        .iter()
        .filter_map(|k| v.get(*k).and_then(Value::as_str))
        .find(|m| !m.is_empty() && *m != "success")
        .unwrap_or(fallback);
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return CloudError::Rejected(format!(
            "Bambu is rate-limiting sign-ins (HTTP 429): {msg}. Wait a few minutes."
        ));
    }
    CloudError::Rejected(format!("{msg} (HTTP {})", status.as_u16()))
}

fn cookie(headers: &reqwest::header::HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .filter_map(|c| c.split(';').next())
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, v)| k.trim() == name && !v.is_empty())
        .map(|(_, v)| v.trim().to_owned())
}

fn parse_device(d: &Value) -> Option<CloudDevice> {
    let s = |k: &str| d.get(k).and_then(Value::as_str).map(str::to_owned);
    Some(CloudDevice {
        dev_id: s("dev_id")?,
        name: s("name").unwrap_or_default(),
        online: d.get("online").and_then(Value::as_bool).unwrap_or(false),
        dev_model_name: s("dev_model_name"),
        dev_product_name: s("dev_product_name"),
        print_status: s("print_status"),
        nozzle_diameter: d.get("nozzle_diameter").and_then(Value::as_f64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacted_debug() {
        let p = Password::new("hunter2".into());
        let t = Token::new("abc.def".into());
        assert!(!format!("{p:?}").contains("hunter2"));
        assert!(!format!("{t:?}").contains("abc"));
    }

    #[test]
    fn issued_prefers_expires_in() {
        let v = serde_json::json!({ "accessToken": "x", "expiresIn": 7_776_000 });
        let it = issued("x".into(), &v);
        let days = (it.expires_at.expect("expiry") - Utc::now()).num_days();
        assert!((89..=90).contains(&days), "{days}");
    }

    #[test]
    fn jwt_exp_is_read() {
        use base64::Engine as _;
        let payload =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"exp":1900000000}"#);
        let t = Token::new(format!("h.{payload}.s"));
        assert_eq!(jwt_expiry(&t).map(|d| d.timestamp()), Some(1_900_000_000));
        assert!(jwt_expiry(&Token::new("opaque".into())).is_none());
    }

    #[test]
    fn device_rows_parse_with_missing_fields() {
        let v = serde_json::json!({
            "dev_id": "03919C000000000", "name": "A1", "online": true,
            "dev_product_name": "A1", "dev_model_name": "N2S"
        });
        let d = parse_device(&v).expect("device");
        assert!(d.online);
        assert_eq!(d.nozzle_diameter, None);
        assert!(parse_device(&serde_json::json!({"name": "x"})).is_none());
    }

    #[test]
    fn error_bodies() {
        assert!(!is_error_body(
            &serde_json::json!({"message":"success","code":null,"error":null})
        ));
        assert!(is_error_body(
            &serde_json::json!({"code": 1, "error": "Incorrect password"})
        ));
        let e = rejection(
            reqwest::StatusCode::BAD_REQUEST,
            &serde_json::json!({"code": 1, "error": "Incorrect password"}),
            "x",
        );
        assert_eq!(e.to_string(), "Incorrect password (HTTP 400)");
    }

    #[test]
    fn token_cookie() {
        let mut h = reqwest::header::HeaderMap::new();
        h.append(
            reqwest::header::SET_COOKIE,
            "a=1; Path=/".parse().expect("hv"),
        );
        h.append(
            reqwest::header::SET_COOKIE,
            "token=tok123; Path=/; HttpOnly".parse().expect("hv"),
        );
        assert_eq!(cookie(&h, "token").as_deref(), Some("tok123"));
    }
}
