use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use std::sync::Arc;
use tracing::{debug, warn};

use crate::error::AppError;
use crate::model::account::Account;
use crate::model::api_token::ApiToken;
use crate::service::account::AccountService;
use crate::service::rewriter::{
    clean_session_id_from_body, detect_client_type, ClientType, Rewriter,
};

const UPSTREAM_BASE: &str = "https://api.anthropic.com";

pub struct GatewayService {
    account_svc: Arc<AccountService>,
    rewriter: Arc<Rewriter>,
}

impl GatewayService {
    pub fn new(
        account_svc: Arc<AccountService>,
        rewriter: Arc<Rewriter>,
    ) -> Self {
        Self {
            account_svc,
            rewriter,
        }
    }

    /// 核心网关逻辑 -- axum handler。
    pub async fn handle_request(&self, req: Request, api_token: Option<&ApiToken>) -> Response {
        match self.handle_request_inner(req, api_token).await {
            Ok(resp) => resp,
            Err(e) => e.into_response(),
        }
    }

    async fn handle_request_inner(&self, req: Request, api_token: Option<&ApiToken>) -> Result<Response, AppError> {
        let method = req.method().clone();
        let path = req.uri().path().to_string();
        let query = req.uri().query().unwrap_or("").to_string();

        // 提取 header
        let headers = extract_headers(req.headers());
        let ua = headers.get("User-Agent").or_else(|| headers.get("user-agent")).cloned().unwrap_or_default();

        // 读取请求体
        let body_bytes = axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024)
            .await
            .map_err(|e| AppError::BadRequest(format!("failed to read body: {}", e)))?;

        // 解析请求体
        let body_map: serde_json::Value = if body_bytes.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_slice(&body_bytes).unwrap_or(serde_json::json!({}))
        };

        // 检测客户端类型
        let client_type = detect_client_type(&ua, &body_map);

        // 生成会话哈希
        let session_hash =
            crate::service::account::generate_session_hash(&ua, &body_map, client_type);

        // 根据令牌限制构建账号过滤条件
        let (allowed_ids, blocked_ids) = if let Some(t) = api_token {
            (t.allowed_account_ids(), t.blocked_account_ids())
        } else {
            (vec![], vec![])
        };

        // 选择账号
        let account = self
            .account_svc
            .select_account(&session_hash, &blocked_ids, &allowed_ids)
            .await
            .map_err(|e| {
                AppError::ServiceUnavailable(format!("no available account: {}", e))
            })?;

        // 获取并发槽位
        let acquired = self
            .account_svc
            .acquire_slot(account.id, account.concurrency)
            .await
            .map_err(|_| AppError::TooManyRequests)?;
        if !acquired {
            return Err(AppError::TooManyRequests);
        }

        // 确保在函数结束后释放槽位
        let account_svc = self.account_svc.clone();
        let account_id_for_release = account.id;
        let _guard = scopeguard::guard((), move |_| {
            let svc = account_svc.clone();
            tokio::spawn(async move {
                svc.release_slot(account_id_for_release).await;
            });
        });

        // 改写请求体
        debug!(
            "request body BEFORE rewrite: {}",
            truncate_body(&body_bytes, 4096)
        );
        let rewritten_body =
            self.rewriter
                .rewrite_body(&body_bytes, &path, &account, client_type);
        debug!(
            "request body AFTER rewrite: {}",
            truncate_body(&rewritten_body, 4096)
        );

        // 重新解析改写后的 body
        let mut rewritten_body_map: serde_json::Value =
            serde_json::from_slice(&rewritten_body).unwrap_or(serde_json::json!({}));

        // 改写 header
        let model_id = body_map
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("");
        let rewritten_headers = self.rewriter.rewrite_headers(
            &headers,
            &account,
            client_type,
            model_id,
            &rewritten_body_map,
        );

        // 清理 body 中的 _session_id 标记并重新序列化
        let final_body = if client_type == ClientType::API {
            clean_session_id_from_body(&mut rewritten_body_map);
            serde_json::to_vec(&rewritten_body_map).unwrap_or_else(|_| rewritten_body.clone())
        } else {
            rewritten_body.clone()
        };

        let mut final_headers = rewritten_headers;
        final_headers.insert(
            "authorization".into(),
            format!("Bearer {}", account.token),
        );

        // 转发到上游
        let resp = self
            .forward_request(
                &method.to_string(),
                &path,
                &query,
                &final_headers,
                &final_body,
                &account,
            )
            .await?;

        Ok(resp)
    }

    async fn forward_request(
        &self,
        method: &str,
        path: &str,
        query: &str,
        headers: &std::collections::HashMap<String, String>,
        body: &[u8],
        account: &Account,
    ) -> Result<Response, AppError> {
        let mut target_url = format!("{}{}", UPSTREAM_BASE, path);
        if !query.is_empty() {
            let q = if query.contains("beta=true") {
                query.to_string()
            } else {
                format!("{}&beta=true", query)
            };
            target_url = format!("{}?{}", target_url, q);
        } else {
            target_url = format!("{}?beta=true", target_url);
        }

        debug!("upstream URL: {}", target_url);

        let client = crate::tlsfp::make_request_client(&account.proxy_url);

        let mut req_builder = match method {
            "GET" => client.get(&target_url),
            "POST" => client.post(&target_url),
            "PUT" => client.put(&target_url),
            "DELETE" => client.delete(&target_url),
            "PATCH" => client.patch(&target_url),
            _ => client.post(&target_url),
        };

        for (k, v) in headers {
            debug!("upstream header: {}: {}", k, v);
            req_builder = req_builder.header(k, v);
        }
        req_builder = req_builder.header("Host", "api.anthropic.com");
        req_builder = req_builder.body(body.to_vec());

        let resp = req_builder
            .send()
            .await
            .map_err(|e| {
                warn!("upstream error for account {}: {}", account.id, e);
                AppError::BadGateway("upstream request failed".into())
            })?;

        let status_code = resp.status().as_u16();
        debug!("upstream response: {}", status_code);

        // 处理限速
        if status_code == 429 {
            self.handle_rate_limit(resp.headers(), account).await;
        }

        // 构建响应
        let mut response_builder = Response::builder().status(
            StatusCode::from_u16(status_code)
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        );

        for (k, v) in resp.headers() {
            response_builder = response_builder.header(k.clone(), v.clone());
        }

        // 流式传输响应体
        let body_stream = resp.bytes_stream();
        let body = Body::from_stream(body_stream);

        response_builder
            .body(body)
            .map_err(|e| AppError::Internal(format!("build response: {}", e)))
    }

    async fn handle_rate_limit(&self, headers: &reqwest::header::HeaderMap, account: &Account) {
        let retry_after = headers
            .get("Retry-After")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let rate_limit_reset = headers
            .get("anthropic-ratelimit-requests-reset")
            .or_else(|| headers.get("anthropic-ratelimit-tokens-reset"))
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        if let Some(retry) = retry_after {
            if let Ok(secs) = retry.parse::<u64>() {
                let reset_at = Utc::now() + chrono::Duration::seconds(secs as i64);
                let _ = self
                    .account_svc
                    .set_rate_limit(account.id, reset_at)
                    .await;
                warn!(
                    "account {} rate limited until {} (Retry-After)",
                    account.id,
                    reset_at.to_rfc3339()
                );
            }
        } else if let Some(reset_str) = rate_limit_reset {
            if let Ok(reset_at) = DateTime::parse_from_rfc3339(&reset_str) {
                let reset_at = reset_at.with_timezone(&Utc);
                let _ = self
                    .account_svc
                    .set_rate_limit(account.id, reset_at)
                    .await;
                warn!(
                    "account {} rate limited until {} (anthropic-ratelimit)",
                    account.id,
                    reset_at.to_rfc3339()
                );
            }
        } else {
            warn!(
                "account {} got 429 without reset headers, not marking rate limited",
                account.id
            );
        }
    }
}

fn extract_headers(headers: &HeaderMap) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for (k, v) in headers {
        if let Ok(val) = v.to_str() {
            map.insert(k.to_string(), val.to_string());
        }
    }
    map
}

fn truncate_body(b: &[u8], max: usize) -> String {
    if b.len() > max {
        format!(
            "{}...(truncated)",
            String::from_utf8_lossy(&b[..max])
        )
    } else {
        String::from_utf8_lossy(b).to_string()
    }
}
