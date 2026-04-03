#![allow(dead_code)]

mod config;
mod error;
mod handler;
mod middleware;
mod model;
mod service;
mod store;
mod tlsfp;

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
            match store::redis::RedisStore::new(&redis_cfg.addr(), &redis_cfg.password, redis_cfg.db)
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
    let gateway_svc = Arc::new(service::gateway::GatewayService::new(
        account_svc.clone(),
        rewriter.clone(),
    ));
    let token_tester = Arc::new(service::oauth::TokenTester::new());

    let app = handler::router::build_router(
        &cfg,
        gateway_svc,
        account_svc,
        token_tester,
        token_store,
    );

    let addr = format!("{}:{}", cfg.server.host, cfg.server.port);
    if cfg.server.tls_cert.is_some() {
        info!("cc2api listening on https://{}", addr);
    } else {
        info!("cc2api listening on http://{}", addr);
    }

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
