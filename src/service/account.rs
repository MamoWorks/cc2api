use chrono::Utc;
use rand::Rng;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;

use crate::error::AppError;
use crate::model::account::Account;
use crate::service::rewriter::ClientType;
use crate::store::account_store::AccountStore;
use crate::store::cache::CacheStore;

const STICKY_SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);

pub struct AccountService {
    store: Arc<AccountStore>,
    cache: Arc<dyn CacheStore>,
}

impl AccountService {
    pub fn new(store: Arc<AccountStore>, cache: Arc<dyn CacheStore>) -> Self {
        Self { store, cache }
    }

    /// 创建新账号并自动生成身份信息。
    pub async fn create_account(&self, a: &mut Account) -> Result<(), AppError> {
        let (device_id, env, prompt, process) =
            crate::model::identity::generate_canonical_identity();
        a.device_id = device_id;
        a.canonical_env = env;
        a.canonical_prompt = prompt;
        a.canonical_process = process;

        if a.status == crate::model::account::AccountStatus::Active && a.status.to_string() == "active" {
            // default already active
        }
        if a.concurrency == 0 {
            a.concurrency = 3;
        }
        if a.priority == 0 {
            a.priority = 50;
        }
        if a.billing_mode == crate::model::account::BillingMode::Strip
            && a.billing_mode.to_string() == "strip"
        {
            // default already strip
        }

        self.store.create(a).await
    }

    pub async fn update_account(&self, a: &Account) -> Result<(), AppError> {
        self.store.update(a).await
    }

    pub async fn delete_account(&self, id: i64) -> Result<(), AppError> {
        self.store.delete(id).await
    }

    pub async fn get_account(&self, id: i64) -> Result<Account, AppError> {
        self.store.get_by_id(id).await
    }

    pub async fn list_accounts(&self) -> Result<Vec<Account>, AppError> {
        self.store.list().await
    }

    pub async fn list_accounts_paged(&self, page: i64, page_size: i64) -> Result<(Vec<Account>, i64), AppError> {
        let total = self.store.count().await?;
        let accounts = self.store.list_paged(page, page_size).await?;
        Ok((accounts, total))
    }

    /// 使用粘性会话为请求选择账号。
    /// `exclude_ids` 为令牌的不可用账号，`allowed_ids` 为令牌的可用账号（空表示不限制）。
    pub async fn select_account(
        &self,
        session_hash: &str,
        exclude_ids: &[i64],
        allowed_ids: &[i64],
    ) -> Result<Account, AppError> {
        // 检查粘性会话
        if !session_hash.is_empty() {
            if let Ok(Some(account_id)) = self.cache.get_session_account_id(session_hash).await {
                if account_id > 0 {
                    if let Ok(account) = self.store.get_by_id(account_id).await {
                        let id_allowed = allowed_ids.is_empty() || allowed_ids.contains(&account_id);
                        if account.is_schedulable()
                            && !exclude_ids.contains(&account_id)
                            && id_allowed
                        {
                            return Ok(account);
                        }
                    }
                    // 过期绑定，删除
                    let _ = self.cache.delete_session(session_hash).await;
                }
            }
        }

        // 获取可调度账号
        let accounts = self.store.list_schedulable().await?;

        // 过滤：排除项 + 可用账号限制
        let candidates: Vec<Account> = accounts
            .into_iter()
            .filter(|a| {
                !exclude_ids.contains(&a.id)
                    && (allowed_ids.is_empty() || allowed_ids.contains(&a.id))
            })
            .collect();

        if candidates.is_empty() {
            return Err(AppError::ServiceUnavailable(
                "no available accounts".into(),
            ));
        }

        // 按优先级分组，同优先级内随机选择
        let selected = select_by_priority(&candidates);

        // 绑定粘性会话
        if !session_hash.is_empty() {
            let _ = self
                .cache
                .set_session_account_id(session_hash, selected.id, STICKY_SESSION_TTL)
                .await;
        }

        Ok(selected)
    }

    /// 尝试获取账号的并发槽位。
    pub async fn acquire_slot(&self, account_id: i64, max: i32) -> Result<bool, AppError> {
        let key = format!("concurrency:account:{}", account_id);
        self.cache
            .acquire_slot(&key, max, Duration::from_secs(300))
            .await
    }

    /// 释放并发槽位。
    pub async fn release_slot(&self, account_id: i64) {
        let key = format!("concurrency:account:{}", account_id);
        self.cache.release_slot(&key).await;
    }

    /// 从 Anthropic API 获取账号用量并缓存到数据库。
    pub async fn refresh_usage(&self, id: i64) -> Result<serde_json::Value, AppError> {
        let account = self.store.get_by_id(id).await?;
        let usage = crate::service::oauth::fetch_usage(&account.token, &account.proxy_url).await?;
        let usage_str = serde_json::to_string(&usage).unwrap_or_else(|_| "{}".into());
        self.store.update_usage(id, &usage_str).await?;
        Ok(usage)
    }

    pub async fn set_rate_limit(
        &self,
        id: i64,
        reset_at: chrono::DateTime<Utc>,
    ) -> Result<(), AppError> {
        self.store.set_rate_limit(id, reset_at).await
    }
}

/// 根据客户端类型创建会话哈希。
/// CC 客户端：使用 metadata.user_id 中的 session_id。
/// API 客户端：使用 sha256(UA + 系统提示词/首条消息 + 小时窗口)。
pub fn generate_session_hash(
    user_agent: &str,
    body: &serde_json::Value,
    client_type: ClientType,
) -> String {
    if client_type == ClientType::ClaudeCode {
        if let Some(metadata) = body.get("metadata").and_then(|m| m.as_object()) {
            if let Some(user_id_str) = metadata.get("user_id").and_then(|u| u.as_str()) {
                // JSON 格式
                if let Ok(uid) = serde_json::from_str::<serde_json::Value>(user_id_str) {
                    if let Some(sid) = uid.get("session_id").and_then(|s| s.as_str()) {
                        if !sid.is_empty() {
                            return sid.to_string();
                        }
                    }
                }
                // 旧格式
                if let Some(idx) = user_id_str.rfind("_session_") {
                    return user_id_str[idx + 9..].to_string();
                }
            }
        }
    }

    // API 模式：UA + 系统提示词/首条消息 + 小时窗口
    let mut content = String::new();

    // Try system prompt first
    match body.get("system") {
        Some(serde_json::Value::String(sys)) => {
            content = sys.clone();
        }
        Some(serde_json::Value::Array(arr)) => {
            for item in arr {
                if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                    content = text.to_string();
                    break;
                }
            }
        }
        _ => {}
    }

    // 回退到首条消息
    if content.is_empty() {
        if let Some(messages) = body.get("messages").and_then(|m| m.as_array()) {
            if let Some(msg) = messages.first().and_then(|m| m.as_object()) {
                match msg.get("content") {
                    Some(serde_json::Value::String(c)) => {
                        content = c.clone();
                    }
                    Some(serde_json::Value::Array(arr)) => {
                        for item in arr {
                            if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                                content = text.to_string();
                                break;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let hour_window = Utc::now().format("%Y-%m-%dT%H").to_string();
    let raw = format!("{}|{}|{}", user_agent, content, hour_window);
    let hash = Sha256::digest(raw.as_bytes());
    hex::encode(&hash[..16])
}

fn select_by_priority(accounts: &[Account]) -> Account {
    if accounts.len() == 1 {
        return accounts[0].clone();
    }

    // 找到最高优先级（最小数值）
    let best_priority = accounts.iter().map(|a| a.priority).min().unwrap_or(50);

    // 收集相同优先级的所有账号
    let best: Vec<&Account> = accounts
        .iter()
        .filter(|a| a.priority == best_priority)
        .collect();

    // 同优先级内随机选择
    let idx = rand::thread_rng().gen_range(0..best.len());
    best[idx].clone()
}
