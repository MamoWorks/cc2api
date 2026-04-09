use redis::AsyncCommands;
use std::time::Duration;

use crate::error::AppError;
use crate::store::cache::CacheStore;

pub struct RedisStore {
    client: redis::aio::ConnectionManager,
}

/// Build a Redis connection URL from discrete components.
///
/// Kept as a pure function so it can be unit-tested without a live Redis server.
/// The database number is encoded exactly once as the URL path.
pub fn build_redis_url(host: &str, port: u16, password: &str, db: i64) -> String {
    if password.is_empty() {
        format!("redis://{}:{}/{}", host, port, db)
    } else {
        format!("redis://:{}@{}:{}/{}", password, host, port, db)
    }
}

impl RedisStore {
    pub async fn new(host: &str, port: u16, password: &str, db: i64) -> Result<Self, AppError> {
        let url = build_redis_url(host, port, password, db);
        let client = redis::Client::open(url)
            .map_err(|e| AppError::Internal(format!("redis open: {}", e)))?;
        let mgr = redis::aio::ConnectionManager::new(client)
            .await
            .map_err(|e| AppError::Internal(format!("redis connect: {}", e)))?;
        Ok(Self { client: mgr })
    }
}

#[axum::async_trait]
impl CacheStore for RedisStore {
    async fn get_session_account_id(&self, session_hash: &str) -> Result<Option<i64>, AppError> {
        let key = format!("session:{}", session_hash);
        let val: Option<String> = self
            .client
            .clone()
            .get(&key)
            .await
            .map_err(|e| AppError::Internal(format!("redis get: {}", e)))?;
        match val {
            Some(s) => {
                let id = s
                    .parse::<i64>()
                    .map_err(|e| AppError::Internal(format!("redis parse: {}", e)))?;
                Ok(Some(id))
            }
            None => Ok(None),
        }
    }

    async fn set_session_account_id(
        &self,
        session_hash: &str,
        account_id: i64,
        ttl: Duration,
    ) -> Result<(), AppError> {
        let key = format!("session:{}", session_hash);
        let _: () = self
            .client
            .clone()
            .set_ex(&key, account_id.to_string(), ttl.as_secs())
            .await
            .map_err(|e| AppError::Internal(format!("redis set: {}", e)))?;
        Ok(())
    }

    async fn delete_session(&self, session_hash: &str) -> Result<(), AppError> {
        let key = format!("session:{}", session_hash);
        let _: () = self
            .client
            .clone()
            .del(&key)
            .await
            .map_err(|e| AppError::Internal(format!("redis del: {}", e)))?;
        Ok(())
    }

    async fn acquire_slot(&self, key: &str, max: i32, ttl: Duration) -> Result<bool, AppError> {
        let mut conn = self.client.clone();
        let val: i64 = conn
            .incr(key, 1i64)
            .await
            .map_err(|e| AppError::Internal(format!("redis incr: {}", e)))?;
        if val == 1 {
            let _: () = conn
                .expire(key, ttl.as_secs() as i64)
                .await
                .unwrap_or(());
        }
        if val > max as i64 {
            let _: () = conn.decr(key, 1i64).await.unwrap_or(());
            return Ok(false);
        }
        Ok(true)
    }

    async fn release_slot(&self, key: &str) {
        let _: Result<(), _> = self.client.clone().decr(key, 1i64).await;
    }

    async fn acquire_lock(
        &self,
        key: &str,
        owner: &str,
        ttl: Duration,
    ) -> Result<bool, AppError> {
        let mut conn = self.client.clone();
        let result: Option<String> = redis::cmd("SET")
            .arg(key)
            .arg(owner)
            .arg("NX")
            .arg("EX")
            .arg(ttl.as_secs().max(1))
            .query_async(&mut conn)
            .await
            .map_err(|e| AppError::Internal(format!("redis lock set: {}", e)))?;
        Ok(result.is_some())
    }

    async fn release_lock(&self, key: &str, owner: &str) {
        let mut conn = self.client.clone();
        let script = redis::Script::new(
            r#"
            if redis.call("GET", KEYS[1]) == ARGV[1] then
                return redis.call("DEL", KEYS[1])
            end
            return 0
            "#,
        );
        let _: Result<i32, _> = script.key(key).arg(owner).invoke_async(&mut conn).await;
    }
}
