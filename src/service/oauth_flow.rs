use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::debug;

use crate::error::AppError;

// ---------------------------------------------------------------------------
// OAuth 常量
// ---------------------------------------------------------------------------

const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const REDIRECT_URI: &str = "https://platform.claude.com/oauth/code/callback";

const SCOPE_FULL: &str =
    "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";
const SCOPE_INFERENCE: &str = "user:inference";

/// 会话 TTL（30 分钟）。
const SESSION_TTL: Duration = Duration::from_secs(30 * 60);

/// Setup-Token 有效期（1 年）。
const SETUP_TOKEN_EXPIRES_IN: i64 = 365 * 24 * 60 * 60;

// ---------------------------------------------------------------------------
// PKCE 工具
// ---------------------------------------------------------------------------

/// base64url 编码（无填充）。
fn base64url_encode(data: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

/// 生成 PKCE code_verifier（32 字节随机 → 43 字符 base64url）。
fn generate_code_verifier() -> String {
    let mut buf = [0u8; 32];
    rand::thread_rng().fill(&mut buf);
    base64url_encode(&buf)
}

/// 计算 S256 code_challenge。
fn generate_code_challenge(verifier: &str) -> String {
    let hash = Sha256::digest(verifier.as_bytes());
    base64url_encode(&hash)
}

/// 生成随机 state。
fn generate_state() -> String {
    let mut buf = [0u8; 32];
    rand::thread_rng().fill(&mut buf);
    base64url_encode(&buf)
}

/// 生成 session_id。
fn generate_session_id() -> String {
    let mut buf = [0u8; 16];
    rand::thread_rng().fill(&mut buf);
    hex::encode(buf)
}

// ---------------------------------------------------------------------------
// 会话存储
// ---------------------------------------------------------------------------

struct OAuthSession {
    state: String,
    code_verifier: String,
    scope: String,
    proxy_url: String,
    created_at: Instant,
}

/// 内存级 OAuth 会话存储，带 TTL 自动清理。
struct SessionStore {
    sessions: Mutex<HashMap<String, OAuthSession>>,
}

impl SessionStore {
    fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }

    fn set(&self, id: &str, session: OAuthSession) {
        let mut map = self.sessions.lock().unwrap();
        // 顺便清理过期会话
        map.retain(|_, s| s.created_at.elapsed() < SESSION_TTL);
        map.insert(id.to_string(), session);
    }

    fn take(&self, id: &str) -> Option<OAuthSession> {
        let mut map = self.sessions.lock().unwrap();
        map.remove(id)
    }
}

// ---------------------------------------------------------------------------
// 请求 / 响应
// ---------------------------------------------------------------------------

/// 生成授权 URL 的请求。
#[derive(Deserialize)]
pub struct GenerateAuthUrlRequest {
    pub proxy_url: Option<String>,
}

/// 生成授权 URL 的响应。
#[derive(Serialize)]
pub struct GenerateAuthUrlResponse {
    pub auth_url: String,
    pub session_id: String,
}

/// 交换 code 的请求。
#[derive(Deserialize)]
pub struct ExchangeCodeRequest {
    pub session_id: String,
    pub code: String,
}

/// 交换 code 的响应。
#[derive(Serialize)]
pub struct ExchangeCodeResponse {
    pub access_token: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub refresh_token: String,
    pub expires_in: i64,
    pub expires_at: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub scope: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub account_uuid: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub organization_uuid: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub email_address: String,
}

/// 平台 token exchange 原始响应。
#[derive(Deserialize)]
struct TokenExchangeResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    expires_in: i64,
    #[serde(default)]
    scope: String,
    account: Option<TokenAccount>,
    organization: Option<TokenOrganization>,
}

#[derive(Deserialize)]
struct TokenAccount {
    uuid: String,
    #[serde(default)]
    email_address: String,
}

#[derive(Deserialize)]
struct TokenOrganization {
    uuid: String,
}

// ---------------------------------------------------------------------------
// OAuthFlowService
// ---------------------------------------------------------------------------

/// 处理 OAuth 授权链接生成和 code 交换。
pub struct OAuthFlowService {
    store: SessionStore,
}

impl OAuthFlowService {
    pub fn new() -> Self {
        Self {
            store: SessionStore::new(),
        }
    }

    /// 生成 OAuth 授权 URL（完整 scope）。
    pub fn generate_auth_url(&self, req: &GenerateAuthUrlRequest) -> GenerateAuthUrlResponse {
        self.build_auth_url(SCOPE_FULL, req.proxy_url.as_deref().unwrap_or(""))
    }

    /// 生成 Setup-Token 授权 URL（仅 user:inference）。
    pub fn generate_setup_token_url(
        &self,
        req: &GenerateAuthUrlRequest,
    ) -> GenerateAuthUrlResponse {
        self.build_auth_url(SCOPE_INFERENCE, req.proxy_url.as_deref().unwrap_or(""))
    }

    /// 交换 code 获取 OAuth token（完整 scope）。
    pub async fn exchange_code(
        &self,
        req: &ExchangeCodeRequest,
    ) -> Result<ExchangeCodeResponse, AppError> {
        self.do_exchange(&req.session_id, &req.code, false).await
    }

    /// 交换 code 获取 Setup-Token。
    pub async fn exchange_setup_token_code(
        &self,
        req: &ExchangeCodeRequest,
    ) -> Result<ExchangeCodeResponse, AppError> {
        self.do_exchange(&req.session_id, &req.code, true).await
    }

    // --- 内部实现 ---

    fn build_auth_url(&self, scope: &str, proxy_url: &str) -> GenerateAuthUrlResponse {
        let state = generate_state();
        let code_verifier = generate_code_verifier();
        let code_challenge = generate_code_challenge(&code_verifier);
        let session_id = generate_session_id();

        self.store.set(
            &session_id,
            OAuthSession {
                state: state.clone(),
                code_verifier,
                scope: scope.to_string(),
                proxy_url: proxy_url.to_string(),
                created_at: Instant::now(),
            },
        );

        let encoded_redirect = percent_encode(REDIRECT_URI);
        let encoded_scope = scope.replace(' ', "+");

        let auth_url = format!(
            "{}?code=true&client_id={}&response_type=code&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}",
            AUTHORIZE_URL, CLIENT_ID, encoded_redirect, encoded_scope, code_challenge, state
        );

        debug!("generated auth URL for session {}", session_id);

        GenerateAuthUrlResponse {
            auth_url,
            session_id,
        }
    }

    async fn do_exchange(
        &self,
        session_id: &str,
        raw_code: &str,
        is_setup_token: bool,
    ) -> Result<ExchangeCodeResponse, AppError> {
        let session = self
            .store
            .take(session_id)
            .ok_or_else(|| AppError::BadRequest("invalid or expired session_id".into()))?;

        if session.created_at.elapsed() >= SESSION_TTL {
            return Err(AppError::BadRequest("session expired".into()));
        }

        // code 可能携带 state：code#state
        let (auth_code, code_state) = if let Some(idx) = raw_code.find('#') {
            (&raw_code[..idx], &raw_code[idx + 1..])
        } else {
            (raw_code, "")
        };

        // 构建 token exchange 请求体
        let mut body = serde_json::json!({
            "grant_type": "authorization_code",
            "code": auth_code,
            "redirect_uri": REDIRECT_URI,
            "client_id": CLIENT_ID,
            "code_verifier": session.code_verifier,
        });

        if !code_state.is_empty() {
            body["state"] = serde_json::Value::String(code_state.to_string());
        } else {
            body["state"] = serde_json::Value::String(session.state.clone());
        }

        if is_setup_token {
            body["expires_in"] = serde_json::json!(SETUP_TOKEN_EXPIRES_IN);
        }

        debug!("exchanging code for session {}", session_id);

        // 发送 token exchange 请求
        let client = crate::tlsfp::make_request_client(&session.proxy_url);
        let resp = client
            .post(TOKEN_URL)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("token exchange request failed: {}", e)))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(AppError::Internal(format!(
                "token exchange failed: status {} {}",
                status, text
            )));
        }

        let token_resp: TokenExchangeResponse = resp
            .json()
            .await
            .map_err(|e| AppError::Internal(format!("token exchange parse failed: {}", e)))?;

        let expires_in = if token_resp.expires_in > 0 {
            token_resp.expires_in
        } else {
            3600
        };
        let expires_at = chrono::Utc::now().timestamp() + expires_in;

        Ok(ExchangeCodeResponse {
            access_token: token_resp.access_token,
            refresh_token: token_resp.refresh_token,
            expires_in,
            expires_at,
            scope: token_resp.scope,
            account_uuid: token_resp
                .account
                .as_ref()
                .map(|a| a.uuid.clone())
                .unwrap_or_default(),
            email_address: token_resp
                .account
                .as_ref()
                .map(|a| a.email_address.clone())
                .unwrap_or_default(),
            organization_uuid: token_resp
                .organization
                .as_ref()
                .map(|o| o.uuid.clone())
                .unwrap_or_default(),
        })
    }
}

/// 简易 percent-encode（仅编码 URL 不安全字符）。
fn percent_encode(input: &str) -> String {
    let mut result = String::with_capacity(input.len() * 3);
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(b as char);
            }
            _ => {
                result.push_str(&format!("%{:02X}", b));
            }
        }
    }
    result
}
