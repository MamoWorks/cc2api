use crate::error::AppError;
use reqwest::Proxy;
use serde_json::Value;

/// 通过轻量级 API 调用验证 Setup Token。
pub struct TokenTester;

impl TokenTester {
    pub fn new() -> Self {
        Self
    }

    /// 通过发送最小消息请求验证 Setup Token 有效性。
    pub async fn test_token(&self, token: &str, proxy_url: &str) -> Result<(), AppError> {
        let body = serde_json::json!({
            "model": "claude-haiku-4-5-20251001",
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "hi"}]
        });

        let mut builder = reqwest::Client::builder();
        if !proxy_url.is_empty() {
            if let Ok(proxy) = Proxy::all(proxy_url) {
                builder = builder.proxy(proxy);
            }
        }
        let client = builder
            .build()
            .map_err(|e| AppError::Internal(format!("http client: {}", e)))?;

        let resp = client
            .post("https://api.anthropic.com/v1/messages?beta=true")
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "oauth-2025-04-20")
            .header("User-Agent", "claude-cli/2.1.89 (external, cli)")
            .header("x-app", "cli")
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("request failed: {}", e)))?;

        if resp.status() != 200 {
            return Err(AppError::Internal(format!(
                "token invalid: status {}",
                resp.status()
            )));
        }
        Ok(())
    }
}

/// 从 Anthropic OAuth API 获取账号用量数据。
pub async fn fetch_usage(token: &str, proxy_url: &str) -> Result<Value, AppError> {
    let mut builder = reqwest::Client::builder();
    if !proxy_url.is_empty() {
        if let Ok(proxy) = Proxy::all(proxy_url) {
            builder = builder.proxy(proxy);
        }
    }
    let client = builder
        .build()
        .map_err(|e| AppError::Internal(format!("http client: {}", e)))?;

    let resp = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("User-Agent", "claude-code/2.1.89 (external, cli)")
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("usage request failed: {}", e)))?;

    if resp.status() != 200 {
        return Err(AppError::Internal(format!(
            "usage fetch failed: status {}",
            resp.status()
        )));
    }

    let data: Value = resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("usage parse failed: {}", e)))?;
    Ok(data)
}
