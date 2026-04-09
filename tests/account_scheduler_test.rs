use chrono::{Duration, Utc};
use sqlx::AnyPool;
use std::sync::Arc;

use claude_code_gateway::error::AppError;
use claude_code_gateway::model::account::{Account, AccountStatus};
use claude_code_gateway::service::account::AccountService;
use claude_code_gateway::store::account_store::AccountStore;
use claude_code_gateway::store::memory::MemoryStore;

/// 创建测试用的临时文件数据库和服务
async fn setup() -> (Arc<AccountStore>, Arc<AccountService>) {
    sqlx::any::install_default_drivers();
    // SQLite in-memory 对 sqlx Any 不兼容，使用临时文件
    let tmp = std::env::temp_dir().join(format!("ccgw_test_{}.db", rand::random::<u64>()));
    let dsn = format!("sqlite:{}?mode=rwc", tmp.display());
    let pool = AnyPool::connect(&dsn)
        .await
        .expect("failed to create sqlite pool");
    claude_code_gateway::store::db::migrate(&pool, "sqlite")
        .await
        .expect("failed to run migrations");

    let store = Arc::new(AccountStore::new(pool, "sqlite".into()));
    let cache = Arc::new(MemoryStore::new());
    let svc = Arc::new(AccountService::new(store.clone(), cache));
    (store, svc)
}

/// 创建一个基础测试账号
async fn create_test_account(svc: &AccountService, email: &str) -> Account {
    let mut account = Account {
        id: 0,
        name: "test".into(),
        email: email.into(),
        status: AccountStatus::Active,
        auth_type: claude_code_gateway::model::account::AccountAuthType::SetupToken,
        setup_token: "sk-ant-oat01-test-token-placeholder-value".into(),
        access_token: String::new(),
        refresh_token: String::new(),
        expires_at: None,
        oauth_refreshed_at: None,
        auth_error: String::new(),
        proxy_url: String::new(),
        device_id: String::new(),
        canonical_env: serde_json::json!({}),
        canonical_prompt: serde_json::json!({}),
        canonical_process: serde_json::json!({}),
        billing_mode: claude_code_gateway::model::account::BillingMode::Strip,
        account_uuid: None,
        organization_uuid: None,
        subscription_type: None,
        concurrency: 3,
        priority: 50,
        rate_limited_at: None,
        rate_limit_reset_at: None,
        disable_reason: String::new(),
        auto_telemetry: false,
        telemetry_count: 0,
        usage_data: serde_json::json!({}),
        usage_fetched_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    svc.create_account(&mut account)
        .await
        .expect("failed to create account");
    account
}

// ─── 429 限流测试 ────────────────────────────────────────────

#[tokio::test]
async fn test_429_disables_account_for_5h() {
    let (_store, svc) = setup().await;
    let account = create_test_account(&svc, "test429@example.com").await;

    // 模拟 429：停用 5 小时，状态保持 Active（靠 rate_limit_reset_at 拦截调度）
    let reset_at = Utc::now() + Duration::hours(5);
    svc.disable_account(account.id, AccountStatus::Active, "429 速率限制", Some(reset_at))
        .await
        .expect("disable_account failed");

    let updated = svc.get_account(account.id).await.expect("get_account failed");
    assert_eq!(updated.status, AccountStatus::Active);
    assert_eq!(updated.disable_reason, "429 速率限制");
    assert!(updated.rate_limit_reset_at.is_some());
    assert!(updated.rate_limited_at.is_some());
    // 应不可调度
    assert!(!updated.is_schedulable());
}

#[tokio::test]
async fn test_429_account_recovers_after_expiry() {
    let (_store, svc) = setup().await;
    let account = create_test_account(&svc, "test429recovery@example.com").await;

    // 设置一个已经过期的 rate_limit_reset_at
    let reset_at = Utc::now() - Duration::seconds(10);
    svc.disable_account(account.id, AccountStatus::Active, "429 速率限制", Some(reset_at))
        .await
        .expect("disable_account failed");

    let updated = svc.get_account(account.id).await.expect("get_account failed");
    // rate_limit_reset_at 已过期，应该可以调度
    assert!(updated.is_schedulable());
}

#[tokio::test]
async fn test_429_account_excluded_from_schedulable_list() {
    let (store, svc) = setup().await;
    let a1 = create_test_account(&svc, "active@example.com").await;
    let a2 = create_test_account(&svc, "limited@example.com").await;

    // a2 被 429 限流
    let reset_at = Utc::now() + Duration::hours(5);
    svc.disable_account(a2.id, AccountStatus::Active, "429 速率限制", Some(reset_at))
        .await
        .unwrap();

    let schedulable = store.list_schedulable().await.unwrap();
    let ids: Vec<i64> = schedulable.iter().map(|a| a.id).collect();
    assert!(ids.contains(&a1.id));
    assert!(!ids.contains(&a2.id));
}

// ─── 403 认证失败测试 ────────────────────────────────────────

#[tokio::test]
async fn test_403_permanently_disables_account() {
    let (_store, svc) = setup().await;
    let account = create_test_account(&svc, "test403@example.com").await;

    svc.disable_account(account.id, AccountStatus::Disabled, "403 认证失败", None)
        .await
        .expect("disable_account failed");

    let updated = svc.get_account(account.id).await.expect("get_account failed");
    assert_eq!(updated.status, AccountStatus::Disabled);
    assert_eq!(updated.disable_reason, "403 认证失败");
    assert!(updated.rate_limit_reset_at.is_none());
    assert!(!updated.is_schedulable());
}

#[tokio::test]
async fn test_403_account_excluded_from_schedulable_list() {
    let (store, svc) = setup().await;
    let a1 = create_test_account(&svc, "ok@example.com").await;
    let a2 = create_test_account(&svc, "forbidden@example.com").await;

    svc.disable_account(a2.id, AccountStatus::Disabled, "403 认证失败", None)
        .await
        .unwrap();

    let schedulable = store.list_schedulable().await.unwrap();
    let ids: Vec<i64> = schedulable.iter().map(|a| a.id).collect();
    assert!(ids.contains(&a1.id));
    assert!(!ids.contains(&a2.id));
}

// ─── 手动停用/启用测试 ──────────────────────────────────────

#[tokio::test]
async fn test_manual_disable_sets_reason() {
    let (_store, svc) = setup().await;
    let account = create_test_account(&svc, "manual@example.com").await;

    svc.disable_account(account.id, AccountStatus::Disabled, "手动停用", None)
        .await
        .unwrap();

    let updated = svc.get_account(account.id).await.unwrap();
    assert_eq!(updated.status, AccountStatus::Disabled);
    assert_eq!(updated.disable_reason, "手动停用");
    assert!(!updated.is_schedulable());
}

#[tokio::test]
async fn test_enable_clears_everything() {
    let (_store, svc) = setup().await;
    let account = create_test_account(&svc, "enable@example.com").await;

    // 先禁用（模拟 429）
    let reset_at = Utc::now() + Duration::hours(5);
    svc.disable_account(account.id, AccountStatus::Active, "429 速率限制", Some(reset_at))
        .await
        .unwrap();

    // 手动启用
    svc.enable_account(account.id).await.unwrap();

    let updated = svc.get_account(account.id).await.unwrap();
    assert_eq!(updated.status, AccountStatus::Active);
    assert_eq!(updated.disable_reason, "");
    assert!(updated.rate_limited_at.is_none());
    assert!(updated.rate_limit_reset_at.is_none());
    assert!(updated.is_schedulable());
}

#[tokio::test]
async fn test_enable_recovers_403_disabled_account() {
    let (_store, svc) = setup().await;
    let account = create_test_account(&svc, "recover403@example.com").await;

    // 403 永久禁用
    svc.disable_account(account.id, AccountStatus::Disabled, "403 认证失败", None)
        .await
        .unwrap();

    // 手动启用
    svc.enable_account(account.id).await.unwrap();

    let updated = svc.get_account(account.id).await.unwrap();
    assert_eq!(updated.status, AccountStatus::Active);
    assert_eq!(updated.disable_reason, "");
    assert!(updated.is_schedulable());
}

// ─── select_account 集成测试 ─────────────────────────────────

#[tokio::test]
async fn test_select_account_skips_429_limited() {
    let (_store, svc) = setup().await;
    let a1 = create_test_account(&svc, "selectable@example.com").await;
    let a2 = create_test_account(&svc, "rate_limited@example.com").await;

    // a2 被限流
    let reset_at = Utc::now() + Duration::hours(5);
    svc.disable_account(a2.id, AccountStatus::Active, "429 速率限制", Some(reset_at))
        .await
        .unwrap();

    // select_account 应该只返回 a1
    let selected = svc.select_account("", &[], &[]).await.unwrap();
    assert_eq!(selected.id, a1.id);
}

#[tokio::test]
async fn test_select_account_skips_403_disabled() {
    let (_store, svc) = setup().await;
    let a1 = create_test_account(&svc, "good@example.com").await;
    let a2 = create_test_account(&svc, "bad@example.com").await;

    // a2 被 403 禁用
    svc.disable_account(a2.id, AccountStatus::Disabled, "403 认证失败", None)
        .await
        .unwrap();

    let selected = svc.select_account("", &[], &[]).await.unwrap();
    assert_eq!(selected.id, a1.id);
}

#[tokio::test]
async fn test_select_account_fails_when_all_disabled() {
    let (_store, svc) = setup().await;
    let a1 = create_test_account(&svc, "only@example.com").await;

    svc.disable_account(a1.id, AccountStatus::Disabled, "403 认证失败", None)
        .await
        .unwrap();

    let result = svc.select_account("", &[], &[]).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        AppError::ServiceUnavailable(_) => {} // 预期
        e => panic!("expected ServiceUnavailable, got: {:?}", e),
    }
}

// ─── disable_reason 覆盖测试 ────────────────────────────────

#[tokio::test]
async fn test_disable_reason_updates_on_repeated_429() {
    let (_store, svc) = setup().await;
    let account = create_test_account(&svc, "repeat429@example.com").await;

    // 第一次 429
    let reset1 = Utc::now() + Duration::hours(5);
    svc.disable_account(account.id, AccountStatus::Active, "429 速率限制", Some(reset1))
        .await
        .unwrap();

    // 第二次 429（重新计时）
    let reset2 = Utc::now() + Duration::hours(5);
    svc.disable_account(account.id, AccountStatus::Active, "429 速率限制", Some(reset2))
        .await
        .unwrap();

    let updated = svc.get_account(account.id).await.unwrap();
    assert_eq!(updated.disable_reason, "429 速率限制");
    assert!(!updated.is_schedulable());
}

#[tokio::test]
async fn test_403_during_429_does_not_escalate() {
    let (_store, svc) = setup().await;
    let account = create_test_account(&svc, "no_escalate@example.com").await;

    // 先 429 限流
    let reset_at = Utc::now() + Duration::hours(5);
    svc.disable_account(account.id, AccountStatus::Active, "429 速率限制", Some(reset_at))
        .await
        .unwrap();

    // 模拟网关逻辑：403 到达时检测到已在限流中，不执行永久停用
    let updated = svc.get_account(account.id).await.unwrap();
    let is_rate_limited = updated
        .rate_limit_reset_at
        .map(|r| Utc::now() < r)
        .unwrap_or(false);
    assert!(is_rate_limited, "account should be rate-limited");

    // 因此不调用 disable_account(Disabled)，账号保持 429 限流状态
    let final_state = svc.get_account(account.id).await.unwrap();
    assert_eq!(final_state.status, AccountStatus::Active);
    assert_eq!(final_state.disable_reason, "429 速率限制");
    assert!(final_state.rate_limit_reset_at.is_some());
}

#[tokio::test]
async fn test_403_after_429_expired_does_disable() {
    let (_store, svc) = setup().await;
    let account = create_test_account(&svc, "expired_then_403@example.com").await;

    // 429 已过期
    let reset_at = Utc::now() - Duration::seconds(10);
    svc.disable_account(account.id, AccountStatus::Active, "429 速率限制", Some(reset_at))
        .await
        .unwrap();

    // 限流已过期，403 应该正常永久停用
    let updated = svc.get_account(account.id).await.unwrap();
    let is_rate_limited = updated
        .rate_limit_reset_at
        .map(|r| Utc::now() < r)
        .unwrap_or(false);
    assert!(!is_rate_limited);

    // 执行 403 停用
    svc.disable_account(account.id, AccountStatus::Disabled, "403 认证失败", None)
        .await
        .unwrap();

    let final_state = svc.get_account(account.id).await.unwrap();
    assert_eq!(final_state.status, AccountStatus::Disabled);
    assert_eq!(final_state.disable_reason, "403 认证失败");
}
