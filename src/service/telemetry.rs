use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use chrono::Utc;
use rand::Rng;
use serde_json::json;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::model::account::{Account, CanonicalEnvData, CanonicalProcessData};
use crate::service::account::AccountService;
use crate::store::account_store::AccountStore;

// ---------------------------------------------------------------------------
// 常量
// ---------------------------------------------------------------------------

const SESSION_TTL: Duration = Duration::from_secs(10 * 60);
const EVENT_BATCH_INTERVAL: Duration = Duration::from_secs(10);
const GROWTHBOOK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const METRICS_INTERVAL: Duration = Duration::from_secs(60);
const TICK_INTERVAL: Duration = Duration::from_secs(1);

const UPSTREAM_BASE: &str = "https://api.anthropic.com";
const GROWTHBOOK_CLIENT_KEY: &str = "sdk-zAZezfDKGoZuXXKe";

// ---------------------------------------------------------------------------
// 遥测路径判断
// ---------------------------------------------------------------------------

/// 判断请求路径是否为遥测端点。
pub fn is_telemetry_path(path: &str) -> bool {
    path.contains("/event_logging/batch")
        || path.starts_with("/api/eval/")
        || path.starts_with("/api/claude_code/metrics")
        || path.starts_with("/api/claude_code/organizations/metrics_enabled")
}

/// 针对 metrics_enabled 返回固定 JSON 响应。
pub fn fake_metrics_enabled_response() -> serde_json::Value {
    json!({"metrics_logging_enabled": true})
}

/// 针对其他遥测端点返回空成功响应。
pub fn fake_telemetry_response() -> serde_json::Value {
    json!({})
}

// ---------------------------------------------------------------------------
// 会话状态
// ---------------------------------------------------------------------------

struct TelemetrySession {
    account: Account,
    token: String,
    expires_at: Instant,
    expires_at_utc: chrono::DateTime<Utc>,
    last_event_batch_at: Instant,
    last_growthbook_at: Option<Instant>,
    last_metrics_at: Instant,
    send_count: i64,
    running: bool,
}

// ---------------------------------------------------------------------------
// TelemetryService
// ---------------------------------------------------------------------------

/// 管理自动遥测会话的后台服务。
pub struct TelemetryService {
    sessions: Arc<Mutex<HashMap<i64, TelemetrySession>>>,
    account_store: Arc<AccountStore>,
    account_svc: Arc<AccountService>,
}

impl TelemetryService {
    pub fn new(account_store: Arc<AccountStore>, account_svc: Arc<AccountService>) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            account_store,
            account_svc,
        }
    }

    /// 查询账号的遥测会话过期时间。
    pub async fn get_session_expires_at(&self, account_id: i64) -> Option<chrono::DateTime<Utc>> {
        let sessions = self.sessions.lock().await;
        sessions.get(&account_id).map(|s| s.expires_at_utc)
    }

    /// 当 /v1/messages 请求到来时调用，激活或续期遥测会话。
    pub async fn activate_session(&self, account: &Account) {
        if !account.auto_telemetry {
            return;
        }

        let token = match self.account_svc.resolve_upstream_token(account.id).await {
            Ok(t) => t,
            Err(e) => {
                warn!(
                    "telemetry: cannot resolve token for account {}: {}",
                    account.id, e
                );
                return;
            }
        };

        let mut sessions = self.sessions.lock().await;
        let now = Instant::now();

        if let Some(session) = sessions.get_mut(&account.id) {
            // 续期
            session.expires_at = now + SESSION_TTL;
            session.expires_at_utc = Utc::now() + chrono::Duration::from_std(SESSION_TTL).unwrap();
            session.token = token;
            session.account = account.clone();
            debug!("telemetry: renewed session for account {}", account.id);
            return;
        }

        // 新建会话
        info!("telemetry: starting session for account {}", account.id);
        let session = TelemetrySession {
            account: account.clone(),
            token,
            expires_at: now + SESSION_TTL,
            expires_at_utc: Utc::now() + chrono::Duration::from_std(SESSION_TTL).unwrap(),
            last_event_batch_at: now - EVENT_BATCH_INTERVAL, // 立即触发首次
            last_growthbook_at: None,
            last_metrics_at: now - METRICS_INTERVAL,
            send_count: 0,
            running: true,
        };
        sessions.insert(account.id, session);

        // 启动后台任务
        let sessions_ref = self.sessions.clone();
        let store_ref = self.account_store.clone();
        let account_id = account.id;
        let proxy_url = account.proxy_url.clone();

        tokio::spawn(async move {
            telemetry_loop(sessions_ref, store_ref, account_id, proxy_url).await;
        });
    }
}

// ---------------------------------------------------------------------------
// 后台循环
// ---------------------------------------------------------------------------

async fn telemetry_loop(
    sessions: Arc<Mutex<HashMap<i64, TelemetrySession>>>,
    store: Arc<AccountStore>,
    account_id: i64,
    proxy_url: String,
) {
    let client = crate::tlsfp::make_request_client(&proxy_url);

    loop {
        tokio::time::sleep(TICK_INTERVAL).await;

        let mut map = sessions.lock().await;
        let session = match map.get_mut(&account_id) {
            Some(s) => s,
            None => break,
        };

        // TTL 过期 → 持久化计数并退出
        if Instant::now() >= session.expires_at {
            let count = session.send_count;
            session.running = false;
            map.remove(&account_id);
            drop(map);
            if count > 0 {
                let _ = store.increment_telemetry_count(account_id, count).await;
            }
            info!(
                "telemetry: session expired for account {}, sent {} requests",
                account_id, count
            );
            break;
        }

        let now = Instant::now();

        // --- event_logging/batch ---
        if now.duration_since(session.last_event_batch_at) >= EVENT_BATCH_INTERVAL {
            let payload = build_event_batch(&session.account);
            let token = session.token.clone();
            let c = client.clone();
            session.last_event_batch_at = now;
            session.send_count += 1;
            drop(map);

            send_telemetry(
                &c,
                &format!("{}/api/event_logging/batch", UPSTREAM_BASE),
                &token,
                &payload,
                &session_ua(&store, account_id).await,
            )
            .await;

            let _ = store.increment_telemetry_count(account_id, 1).await;
            continue;
        }

        // --- GrowthBook eval ---
        let should_gb = match session.last_growthbook_at {
            None => true,
            Some(t) => now.duration_since(t) >= GROWTHBOOK_INTERVAL,
        };
        if should_gb {
            let payload = build_growthbook_eval(&session.account);
            let token = session.token.clone();
            let c = client.clone();
            session.last_growthbook_at = Some(now);
            session.send_count += 1;
            drop(map);

            send_telemetry(
                &c,
                &format!("{}/api/eval/{}", UPSTREAM_BASE, GROWTHBOOK_CLIENT_KEY),
                &token,
                &payload,
                &session_ua(&store, account_id).await,
            )
            .await;

            let _ = store.increment_telemetry_count(account_id, 1).await;
            continue;
        }

        // --- metrics (跳过：该端点不支持 OAuth 认证) ---

        drop(map);
    }
}

/// 从 account store 获取最新的 UA 版本号。
async fn session_ua(store: &Arc<AccountStore>, account_id: i64) -> String {
    let version = store
        .get_by_id(account_id)
        .await
        .ok()
        .and_then(|a| serde_json::from_value::<CanonicalEnvData>(a.canonical_env).ok())
        .map(|e| e.version)
        .unwrap_or_else(|| "2.1.81".into());
    format!("claude-code/{} (external, cli)", version)
}

// ---------------------------------------------------------------------------
// HTTP 发送
// ---------------------------------------------------------------------------

async fn send_telemetry(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    body: &serde_json::Value,
    user_agent: &str,
) {
    let result = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("User-Agent", user_agent)
        .header("x-service-name", "claude-code")
        .header("Authorization", format!("Bearer {}", token))
        .json(body)
        .send()
        .await;

    match result {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                debug!("telemetry: {} → {}", url, status);
            } else {
                let text = resp.text().await.unwrap_or_default();
                warn!("telemetry: {} → {} {}", url, status, text);
            }
        }
        Err(e) => {
            warn!("telemetry: {} failed: {}", url, e);
        }
    }
}

// ---------------------------------------------------------------------------
// 请求体构造
// ---------------------------------------------------------------------------

fn parse_env(account: &Account) -> CanonicalEnvData {
    serde_json::from_value(account.canonical_env.clone()).unwrap_or_default()
}

fn parse_process(account: &Account) -> CanonicalProcessData {
    serde_json::from_value(account.canonical_process.clone()).unwrap_or_default()
}

fn random_in_range(min: i64, max: i64) -> i64 {
    if max <= min {
        return min;
    }
    rand::thread_rng().gen_range(min..max)
}

fn build_process_json(proc: &CanonicalProcessData) -> serde_json::Value {
    json!({
        "constrainedMemory": proc.constrained_memory,
        "rss": random_in_range(proc.rss_range[0], proc.rss_range[1]),
        "heapTotal": random_in_range(proc.heap_total_range[0], proc.heap_total_range[1]),
        "heapUsed": random_in_range(proc.heap_used_range[0], proc.heap_used_range[1]),
    })
}

fn derive_account_uuid(account: &Account) -> String {
    account.account_uuid.clone().unwrap_or_else(|| {
        use sha2::{Digest, Sha256};
        let seed = if account.email.is_empty() {
            format!("account-{}", account.id)
        } else {
            account.email.clone()
        };
        let hash = Sha256::digest(seed.as_bytes());
        format!(
            "{}-{}-{}-{}-{}",
            hex::encode(&hash[0..4]),
            hex::encode(&hash[4..6]),
            hex::encode(&hash[6..8]),
            hex::encode(&hash[8..10]),
            hex::encode(&hash[10..16])
        )
    })
}

/// 构造 /api/event_logging/batch 请求体。
fn build_event_batch(account: &Account) -> serde_json::Value {
    let env = parse_env(account);
    let proc = parse_process(account);
    let account_uuid = derive_account_uuid(account);
    let session_id = uuid::Uuid::new_v4().to_string();
    let process_b64 = {
        let p = build_process_json(&proc);
        let bytes = serde_json::to_vec(&p).unwrap_or_default();
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    };

    let env_obj = json!({
        "platform": env.platform,
        "platform_raw": env.platform_raw,
        "arch": env.arch,
        "node_version": env.node_version,
        "terminal": env.terminal,
        "package_managers": env.package_managers,
        "runtimes": env.runtimes,
        "is_running_with_bun": false,
        "is_ci": false,
        "is_claubbit": false,
        "is_claude_code_remote": false,
        "is_local_agent_mode": false,
        "is_conductor": false,
        "is_github_action": false,
        "is_claude_code_action": false,
        "is_claude_ai_auth": env.is_claude_ai_auth,
        "version": env.version,
        "version_base": env.version_base,
        "build_time": env.build_time,
        "deployment_environment": env.deployment_environment,
        "vcs": env.vcs,
    });

    let mut auth = json!({});
    auth["account_uuid"] = json!(account_uuid);
    if let Some(ref org) = account.organization_uuid {
        auth["organization_uuid"] = json!(org);
    }

    let event = json!({
        "event_type": "ClaudeCodeInternalEvent",
        "event_data": {
            "event_id": uuid::Uuid::new_v4().to_string(),
            "event_name": "tengu_api_success",
            "client_timestamp": Utc::now().to_rfc3339(),
            "device_id": account.device_id,
            "email": account.email,
            "session_id": session_id,
            "model": "claude-sonnet-4-20250514",
            "auth": auth,
            "env": env_obj,
            "process": process_b64,
        }
    });

    json!({ "events": [event] })
}

/// 构造 /api/eval/{clientKey} 请求体（GrowthBook remote eval）。
fn build_growthbook_eval(account: &Account) -> serde_json::Value {
    let env = parse_env(account);
    let account_uuid = derive_account_uuid(account);

    let mut attrs = json!({
        "id": account.device_id,
        "deviceID": account.device_id,
        "platform": env.platform,
        "appVersion": env.version,
        "email": account.email,
        "accountUUID": account_uuid,
    });

    if let Some(ref org) = account.organization_uuid {
        attrs["organizationUUID"] = json!(org);
    }
    if let Some(ref sub) = account.subscription_type {
        attrs["subscriptionType"] = json!(sub);
    }

    json!({
        "attributes": attrs,
        "forcedFeatures": {},
    })
}

/// 构造 /api/claude_code/metrics 请求体。
fn build_metrics(account: &Account) -> serde_json::Value {
    let env = parse_env(account);
    let os_type = match env.platform.as_str() {
        "darwin" => "Darwin",
        "win32" => "Windows",
        _ => "Linux",
    };

    let mut resource = json!({
        "service.name": "claude-code",
        "service.version": env.version,
        "os.type": os_type,
        "host.arch": env.arch,
        "aggregation.temporality": "delta",
        "user.customer_type": "claude_ai",
    });

    if let Some(ref sub) = account.subscription_type {
        resource["user.subscription_type"] = json!(sub);
    }

    json!({
        "resource_attributes": resource,
        "metrics": [],
    })
}
