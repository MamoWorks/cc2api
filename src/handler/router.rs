use axum::extract::{Path, Query, Request, State};
use serde::Deserialize;
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use rust_embed::Embed;
use std::sync::Arc;

use crate::config::Config;
use crate::error::AppError;
use crate::middleware::auth::{admin_auth, gateway_auth};
use crate::model::account::{Account, AccountStatus};
use crate::model::api_token::{self, ApiToken};
use crate::service::account::AccountService;
use crate::service::gateway::GatewayService;
use crate::service::oauth::TokenTester;
use crate::store::token_store::TokenStore;

#[derive(Clone)]
pub struct AppState {
    pub gateway_svc: Arc<GatewayService>,
    pub account_svc: Arc<AccountService>,
    pub token_tester: Arc<TokenTester>,
    pub token_store: Arc<TokenStore>,
    pub admin_password: String,
}

pub fn build_router(
    cfg: &Config,
    gateway_svc: Arc<GatewayService>,
    account_svc: Arc<AccountService>,
    token_tester: Arc<TokenTester>,
    token_store: Arc<TokenStore>,
) -> Router {
    let state = AppState {
        gateway_svc,
        account_svc,
        token_tester,
        token_store,
        admin_password: cfg.admin.password.clone(),
    };

    let token_store_for_gateway = state.token_store.clone();
    let token_store_for_models = state.token_store.clone();
    let admin_password = state.admin_password.clone();

    // 网关路由（令牌认证）
    let gateway_routes = Router::new()
        .route("/v1/messages", post(gateway_handler))
        .route("/v1/messages/*rest", post(gateway_handler))
        .route("/v1/*rest", post(gateway_handler).get(gateway_handler))
        .route("/api/*rest", post(gateway_handler).get(gateway_handler))
        .layer(middleware::from_fn(move |req, next: Next| {
            let store = token_store_for_gateway.clone();
            gateway_auth(store, req, next)
        }))
        .with_state(state.clone());

    // 模型列表（令牌认证）
    let models_route = Router::new()
        .route("/v1/models", get(list_models))
        .layer(middleware::from_fn(move |req, next: Next| {
            let store = token_store_for_models.clone();
            gateway_auth(store, req, next)
        }))
        .with_state(state.clone());

    // 管理路由（密码认证）
    let admin_routes = Router::new()
        .route("/admin/accounts", get(list_accounts).post(create_account))
        .route(
            "/admin/accounts/:id",
            put(update_account).delete(delete_account),
        )
        .route("/admin/accounts/:id/test", post(test_account))
        .route("/admin/accounts/:id/usage", post(refresh_usage))
        .route("/admin/tokens", get(list_tokens).post(create_token))
        .route(
            "/admin/tokens/:id",
            put(update_token).delete(delete_token_handler),
        )
        .route("/admin/dashboard", get(get_dashboard))
        .layer(middleware::from_fn(move |req, next: Next| {
            let pwd = admin_password.clone();
            admin_auth(pwd, req, next)
        }))
        .with_state(state.clone());

    // 健康检查
    let health = Router::new().route("/_health", get(health_handler));

    // 组合所有路由
    let mut app = Router::new()
        .merge(health)
        .merge(models_route)
        .merge(admin_routes)
        .merge(gateway_routes);

    // 前端静态资源（编译时嵌入二进制）
    app = app.fallback(static_handler);

    app
}

// --- Handlers ---

async fn health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "ok"}))
}

async fn gateway_handler(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    // 从请求扩展中取出已验证的 ApiToken
    let api_token = req.extensions().get::<ApiToken>().cloned();
    state.gateway_svc.handle_request(req, api_token.as_ref()).await
}

// --- Account Handlers ---

#[derive(Deserialize)]
struct PageQuery {
    page: Option<i64>,
    page_size: Option<i64>,
}

async fn list_accounts(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(12).clamp(1, 100);
    let (accounts, total) = state.account_svc.list_accounts_paged(page, page_size).await?;
    let total_pages = (total + page_size - 1) / page_size;
    Ok(Json(serde_json::json!({
        "data": accounts,
        "total": total,
        "page": page,
        "page_size": page_size,
        "total_pages": total_pages,
    })))
}

#[derive(Deserialize)]
struct CreateAccountRequest {
    name: Option<String>,
    email: String,
    token: String,
    proxy_url: Option<String>,
    billing_mode: Option<String>,
    concurrency: Option<i32>,
    priority: Option<i32>,
}

async fn create_account(
    State(state): State<AppState>,
    Json(req): Json<CreateAccountRequest>,
) -> Result<(StatusCode, Json<Account>), AppError> {
    if req.token.is_empty() || req.email.is_empty() {
        return Err(AppError::BadRequest(
            "token and email are required".into(),
        ));
    }
    let mut account = Account {
        id: 0,
        name: req.name.unwrap_or_default(),
        email: req.email,
        status: AccountStatus::Active,
        token: req.token,
        proxy_url: req.proxy_url.unwrap_or_default(),
        device_id: String::new(),
        canonical_env: serde_json::json!({}),
        canonical_prompt: serde_json::json!({}),
        canonical_process: serde_json::json!({}),
        billing_mode: req.billing_mode.unwrap_or_else(|| "strip".into()).into(),
        concurrency: req.concurrency.unwrap_or(3),
        priority: req.priority.unwrap_or(50),
        rate_limited_at: None,
        rate_limit_reset_at: None,
        usage_data: serde_json::json!({}),
        usage_fetched_at: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    state.account_svc.create_account(&mut account).await?;
    Ok((StatusCode::CREATED, Json(account)))
}

async fn update_account(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(updates): Json<serde_json::Value>,
) -> Result<Json<Account>, AppError> {
    let mut existing = state.account_svc.get_account(id).await?;

    if let Some(name) = updates.get("name").and_then(|v| v.as_str()) {
        if !name.is_empty() {
            existing.name = name.to_string();
        }
    }
    if let Some(email) = updates.get("email").and_then(|v| v.as_str()) {
        if !email.is_empty() {
            existing.email = email.to_string();
        }
    }
    if let Some(token) = updates.get("token").and_then(|v| v.as_str()) {
        if !token.is_empty() {
            existing.token = token.to_string();
        }
    }
    if let Some(proxy_url) = updates.get("proxy_url").and_then(|v| v.as_str()) {
        if !proxy_url.is_empty() {
            existing.proxy_url = proxy_url.to_string();
        }
    }
    if let Some(concurrency) = updates.get("concurrency").and_then(|v| v.as_i64()) {
        if concurrency > 0 {
            existing.concurrency = concurrency as i32;
        }
    }
    if let Some(priority) = updates.get("priority").and_then(|v| v.as_i64()) {
        if priority > 0 {
            existing.priority = priority as i32;
        }
    }
    if let Some(status) = updates.get("status").and_then(|v| v.as_str()) {
        if !status.is_empty() {
            existing.status = status.to_string().into();
        }
    }
    if let Some(billing_mode) = updates.get("billing_mode").and_then(|v| v.as_str()) {
        if !billing_mode.is_empty() {
            existing.billing_mode = billing_mode.to_string().into();
        }
    }

    state.account_svc.update_account(&existing).await?;
    Ok(Json(existing))
}

async fn delete_account(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, AppError> {
    state.account_svc.delete_account(id).await?;
    Ok(Json(serde_json::json!({"status": "deleted"})))
}

async fn test_account(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, AppError> {
    let account = state.account_svc.get_account(id).await?;
    match state
        .token_tester
        .test_token(&account.token, &account.proxy_url)
        .await
    {
        Ok(()) => Ok(Json(serde_json::json!({"status": "ok"}))),
        Err(e) => Ok(Json(
            serde_json::json!({"status": "error", "message": e.to_string()}),
        )),
    }
}

async fn refresh_usage(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, AppError> {
    match state.account_svc.refresh_usage(id).await {
        Ok(usage) => Ok(Json(serde_json::json!({"status": "ok", "usage": usage}))),
        Err(e) => Ok(Json(
            serde_json::json!({"status": "error", "message": e.to_string()}),
        )),
    }
}

// --- Token Handlers ---

async fn list_tokens(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).clamp(1, 100);
    let total = state.token_store.count().await?;
    let tokens = state.token_store.list_paged(page, page_size).await?;
    let total_pages = (total + page_size - 1) / page_size;
    Ok(Json(serde_json::json!({
        "data": tokens,
        "total": total,
        "page": page,
        "page_size": page_size,
        "total_pages": total_pages,
    })))
}

#[derive(Deserialize)]
struct CreateTokenRequest {
    name: Option<String>,
    allowed_accounts: Option<String>,
    blocked_accounts: Option<String>,
}

async fn create_token(
    State(state): State<AppState>,
    Json(req): Json<CreateTokenRequest>,
) -> Result<(StatusCode, Json<ApiToken>), AppError> {
    let mut token = ApiToken {
        id: 0,
        name: req.name.unwrap_or_default(),
        token: api_token::generate_token(),
        allowed_accounts: req.allowed_accounts.unwrap_or_default(),
        blocked_accounts: req.blocked_accounts.unwrap_or_default(),
        status: api_token::ApiTokenStatus::Active,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    state.token_store.create(&mut token).await?;
    Ok((StatusCode::CREATED, Json(token)))
}

async fn update_token(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(updates): Json<serde_json::Value>,
) -> Result<Json<ApiToken>, AppError> {
    let mut existing = state.token_store.get_by_id(id).await?;

    if let Some(name) = updates.get("name").and_then(|v| v.as_str()) {
        existing.name = name.to_string();
    }
    if let Some(allowed) = updates.get("allowed_accounts").and_then(|v| v.as_str()) {
        existing.allowed_accounts = allowed.to_string();
    }
    if let Some(blocked) = updates.get("blocked_accounts").and_then(|v| v.as_str()) {
        existing.blocked_accounts = blocked.to_string();
    }
    if let Some(status) = updates.get("status").and_then(|v| v.as_str()) {
        if !status.is_empty() {
            existing.status = status.to_string().into();
        }
    }

    state.token_store.update(&existing).await?;
    Ok(Json(existing))
}

async fn delete_token_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, AppError> {
    state.token_store.delete(id).await?;
    Ok(Json(serde_json::json!({"status": "deleted"})))
}

// --- Dashboard & Models ---

async fn get_dashboard(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    let accounts = state.account_svc.list_accounts().await?;
    let token_count = state.token_store.count().await.unwrap_or(0);

    let mut active = 0;
    let mut err_count = 0;
    let mut disabled = 0;
    for a in &accounts {
        match a.status {
            AccountStatus::Active => active += 1,
            AccountStatus::Error => err_count += 1,
            AccountStatus::Disabled => disabled += 1,
        }
    }

    Ok(Json(serde_json::json!({
        "accounts": {
            "total": accounts.len(),
            "active": active,
            "error": err_count,
            "disabled": disabled,
        },
        "tokens": token_count,
    })))
}

async fn list_models() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "data": [
            {"id": "claude-opus-4-5-20251101", "type": "model", "display_name": "Claude Opus 4.5", "created_at": "2025-11-01T00:00:00Z"},
            {"id": "claude-opus-4-6", "type": "model", "display_name": "Claude Opus 4.6", "created_at": "2026-02-06T00:00:00Z"},
            {"id": "claude-sonnet-4-6", "type": "model", "display_name": "Claude Sonnet 4.6", "created_at": "2026-02-18T00:00:00Z"},
            {"id": "claude-sonnet-4-5-20250929", "type": "model", "display_name": "Claude Sonnet 4.5", "created_at": "2025-09-29T00:00:00Z"},
            {"id": "claude-haiku-4-5-20251001", "type": "model", "display_name": "Claude Haiku 4.5", "created_at": "2025-10-01T00:00:00Z"},
        ],
        "object": "list",
    }))
}

// --- 内嵌前端静态资源 ---

#[derive(Embed)]
#[folder = "web/dist"]
struct Assets;

/// 根据请求路径返回内嵌的静态文件，未匹配则返回 index.html（SPA 路由）
async fn static_handler(req: Request) -> impl IntoResponse {
    let path = req.uri().path().trim_start_matches('/');
    // 优先匹配静态文件
    if let Some(file) = Assets::get(path) {
        let mime = mime_from_path(path);
        return Response::builder()
            .header("content-type", mime)
            .body(axum::body::Body::from(file.data.to_vec()))
            .unwrap();
    }
    // SPA fallback: 返回 index.html
    match Assets::get("index.html") {
        Some(index) => Response::builder()
            .header("content-type", "text/html")
            .body(axum::body::Body::from(index.data.to_vec()))
            .unwrap(),
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(axum::body::Body::from("frontend not built"))
            .unwrap(),
    }
}

fn mime_from_path(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html",
        Some("css") => "text/css",
        Some("js") => "application/javascript",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        _ => "application/octet-stream",
    }
}
