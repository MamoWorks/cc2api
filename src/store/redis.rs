use redis::AsyncCommands;
use std::time::Duration;

use crate::error::AppError;
use crate::store::cache::CacheStore;

pub struct RedisStore {
    client: redis::aio::ConnectionManager,
}

impl RedisStore {
    pub async fn new(addr: &str, password: &str, db: i64) -> Result<Self, AppError> {
        let url = if password.is_empty() {
            format!("{}/{}", addr, db)
        } else {
            // Rebuild URL with password
            let base = addr.trim_start_matches("redis://");
            format!("redis://:{}@{}/{}", password, base, db)
        };
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
}
