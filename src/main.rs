use claude_code_gateway::config;
use claude_code_gateway::handler;
use claude_code_gateway::service;
use claude_code_gateway::store;

use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() {
    let cfg = config::Config::load();

    // 初始化日志
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| cfg.log_level.clone().into()),
        )
        .init();

    // 注册 sqlx Any 驱动
    sqlx::any::install_default_drivers();

    // 初始化数据库
    let driver = cfg.database.driver();
    let dsn = cfg.database.dsn();
    info!("database: {} ({})", driver, dsn);

    let pool = store::db::init_db(&driver, &dsn)
        .await
        .expect("init db failed");
    store::db::migrate(&pool, &driver)
        .await
        .expect("migrate failed");

    // 缓存：优先 Redis，回退内存
    let cache: Arc<dyn store::cache::CacheStore> = match &cfg.redis {
        Some(redis_cfg) => {
            match store::redis::RedisStore::new(
                &redis_cfg.host,
                redis_cfg.port,
                &redis_cfg.password,
                redis_cfg.db,
            )
                .await
            {
                Ok(r) => {
                    info!("using redis cache");
                    Arc::new(r)
                }
                Err(e) => {
                    info!("redis unavailable ({}), using in-memory cache", e);
                    Arc::new(store::memory::MemoryStore::new())
                }
            }
        }
        None => {
            info!("no redis configured, using in-memory cache");
            Arc::new(store::memory::MemoryStore::new())
        }
    };

    let account_store = Arc::new(store::account_store::AccountStore::new(pool.clone(), driver.clone()));
    let token_store = Arc::new(store::token_store::TokenStore::new(pool.clone(), driver.clone()));

    let account_svc = Arc::new(service::account::AccountService::new(
        account_store.clone(),
        cache.clone(),
    ));
    let rewriter = Arc::new(service::rewriter::Rewriter::new());
    let telemetry_svc = Arc::new(service::telemetry::TelemetryService::new(
        account_store.clone(),
        account_svc.clone(),
    ));
    let gateway_svc = Arc::new(service::gateway::GatewayService::new(
        account_svc.clone(),
        rewriter.clone(),
        telemetry_svc.clone(),
    ));
    let token_tester = Arc::new(service::oauth::TokenTester::new());
    let oauth_flow_svc = Arc::new(service::oauth_flow::OAuthFlowService::new());

    let app = handler::router::build_router(
        &cfg,
        gateway_svc,
        account_svc,
        token_tester,
        token_store,
        oauth_flow_svc,
        telemetry_svc,
    );

    let addr = format!("{}:{}", cfg.server.host, cfg.server.port);
    if cfg.server.tls_cert.is_some() {
        info!("claude-code-gateway listening on https://{}", addr);
    } else {
        info!("claude-code-gateway listening on http://{}", addr);
    }

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
