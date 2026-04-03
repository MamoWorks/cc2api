use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::Mutex;

use crate::error::AppError;
use crate::store::cache::CacheStore;

struct SessionEntry {
    account_id: i64,
    expires_at: tokio::time::Instant,
}

struct LockEntry {
    owner: String,
    expires_at: tokio::time::Instant,
}

pub struct MemoryStore {
    sessions: Mutex<HashMap<String, SessionEntry>>,
    slots: Mutex<HashMap<String, i64>>,
    locks: Mutex<HashMap<String, LockEntry>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            slots: Mutex::new(HashMap::new()),
            locks: Mutex::new(HashMap::new()),
        }
    }
}

#[axum::async_trait]
impl CacheStore for MemoryStore {
    async fn get_session_account_id(&self, session_hash: &str) -> Result<Option<i64>, AppError> {
        let mut sessions = self.sessions.lock().await;
        let key = format!("session:{}", session_hash);
        if let Some(entry) = sessions.get(&key) {
            if tokio::time::Instant::now() > entry.expires_at {
                sessions.remove(&key);
                return Ok(None);
            }
            return Ok(Some(entry.account_id));
        }
        Ok(None)
    }

    async fn set_session_account_id(
        &self,
        session_hash: &str,
        account_id: i64,
        ttl: Duration,
    ) -> Result<(), AppError> {
        let mut sessions = self.sessions.lock().await;
        let key = format!("session:{}", session_hash);
        sessions.insert(
            key,
            SessionEntry {
                account_id,
                expires_at: tokio::time::Instant::now() + ttl,
            },
        );
        Ok(())
    }

    async fn delete_session(&self, session_hash: &str) -> Result<(), AppError> {
        let mut sessions = self.sessions.lock().await;
        sessions.remove(&format!("session:{}", session_hash));
        Ok(())
    }

    async fn acquire_slot(&self, key: &str, max: i32, _ttl: Duration) -> Result<bool, AppError> {
        let mut slots = self.slots.lock().await;
        let val = slots.entry(key.to_string()).or_insert(0);
        *val += 1;
        if *val > max as i64 {
            *val -= 1;
            return Ok(false);
        }
        Ok(true)
    }

    async fn release_slot(&self, key: &str) {
        let mut slots = self.slots.lock().await;
        if let Some(val) = slots.get_mut(key) {
            if *val > 0 {
                *val -= 1;
            }
        }
    }

    async fn acquire_lock(
        &self,
        key: &str,
        owner: &str,
        ttl: Duration,
    ) -> Result<bool, AppError> {
        let mut locks = self.locks.lock().await;
        let now = tokio::time::Instant::now();
        if let Some(existing) = locks.get(key) {
            if now <= existing.expires_at {
                return Ok(false);
            }
        }
        locks.insert(
            key.to_string(),
            LockEntry {
                owner: owner.to_string(),
                expires_at: now + ttl,
            },
        );
        Ok(true)
    }

    async fn release_lock(&self, key: &str, owner: &str) {
        let mut locks = self.locks.lock().await;
        if let Some(existing) = locks.get(key) {
            if existing.owner == owner {
                locks.remove(key);
            }
        }
    }
}
