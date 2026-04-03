use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::Value;
use sqlx::any::AnyRow;
use sqlx::AnyPool;
use sqlx::Row;

use crate::error::AppError;
use crate::model::account::{Account, AccountStatus};

pub struct AccountStore {
    pool: AnyPool,
    driver: String,
}

impl AccountStore {
    pub fn new(pool: AnyPool, driver: String) -> Self {
        Self { pool, driver }
    }

    fn now_expr(&self) -> &str {
        if self.driver == "sqlite" {
            "strftime('%Y-%m-%dT%H:%M:%SZ','now')"
        } else {
            "NOW()"
        }
    }

    fn fmt_time(&self, t: DateTime<Utc>) -> String {
        t.format("%Y-%m-%dT%H:%M:%SZ").to_string()
    }

    fn parse_time(row: &AnyRow, col: &str) -> DateTime<Utc> {
        // SQLite returns string, Postgres returns native
        if let Ok(s) = row.try_get::<String, _>(col) {
            NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%SZ")
                .map(|n| n.and_utc())
                .unwrap_or_default()
        } else {
            Utc::now()
        }
    }

    fn parse_optional_time(row: &AnyRow, col: &str) -> Option<DateTime<Utc>> {
        if let Ok(s) = row.try_get::<Option<String>, _>(col) {
            s.and_then(|s| {
                NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%SZ")
                    .map(|n| n.and_utc())
                    .ok()
            })
        } else {
            None
        }
    }

    fn parse_json(row: &AnyRow, col: &str) -> Value {
        if let Ok(s) = row.try_get::<String, _>(col) {
            serde_json::from_str(&s).unwrap_or_else(|_| Value::Object(Default::default()))
        } else {
            Value::Object(Default::default())
        }
    }

    fn row_to_account(row: &AnyRow) -> Account {
        Account {
            id: row.try_get::<i64, _>("id").unwrap_or_default(),
            name: row.try_get::<String, _>("name").unwrap_or_default(),
            email: row.try_get::<String, _>("email").unwrap_or_default(),
            status: row
                .try_get::<String, _>("status")
                .unwrap_or_else(|_| "active".into())
                .into(),
            token: row.try_get::<String, _>("token").unwrap_or_default(),
            proxy_url: row.try_get::<String, _>("proxy_url").unwrap_or_default(),
            device_id: row.try_get::<String, _>("device_id").unwrap_or_default(),
            canonical_env: Self::parse_json(row, "canonical_env"),
            canonical_prompt: Self::parse_json(row, "canonical_prompt_env"),
            canonical_process: Self::parse_json(row, "canonical_process"),
            billing_mode: row
                .try_get::<String, _>("billing_mode")
                .unwrap_or_else(|_| "strip".into())
                .into(),
            concurrency: row.try_get::<i32, _>("concurrency").unwrap_or(3),
            priority: row.try_get::<i32, _>("priority").unwrap_or(50),
            rate_limited_at: Self::parse_optional_time(row, "rate_limited_at"),
            rate_limit_reset_at: Self::parse_optional_time(row, "rate_limit_reset_at"),
            usage_data: Self::parse_json(row, "usage_data"),
            usage_fetched_at: Self::parse_optional_time(row, "usage_fetched_at"),
            created_at: Self::parse_time(row, "created_at"),
            updated_at: Self::parse_time(row, "updated_at"),
        }
    }

    pub async fn create(&self, a: &mut Account) -> Result<(), AppError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) as cnt FROM accounts WHERE email=$1",
        )
        .bind(&a.email)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        if count > 0 {
            return Err(AppError::BadRequest(format!(
                "email {} already exists",
                a.email
            )));
        }

        let env_str = serde_json::to_string(&a.canonical_env).unwrap_or_else(|_| "{}".into());
        let prompt_str =
            serde_json::to_string(&a.canonical_prompt).unwrap_or_else(|_| "{}".into());
        let process_str =
            serde_json::to_string(&a.canonical_process).unwrap_or_else(|_| "{}".into());

        let row: AnyRow = sqlx::query(
            r#"INSERT INTO accounts (name, email, status, token, proxy_url,
                device_id, canonical_env, canonical_prompt_env, canonical_process,
                billing_mode, concurrency, priority)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
            RETURNING id, created_at, updated_at"#,
        )
        .bind(&a.name)
        .bind(&a.email)
        .bind(a.status.to_string())
        .bind(&a.token)
        .bind(&a.proxy_url)
        .bind(&a.device_id)
        .bind(&env_str)
        .bind(&prompt_str)
        .bind(&process_str)
        .bind(a.billing_mode.to_string())
        .bind(a.concurrency)
        .bind(a.priority)
        .fetch_one(&self.pool)
        .await?;

        a.id = row.try_get::<i64, _>("id").unwrap_or_default();
        a.created_at = Self::parse_time(&row, "created_at");
        a.updated_at = Self::parse_time(&row, "updated_at");
        Ok(())
    }

    pub async fn update(&self, a: &Account) -> Result<(), AppError> {
        let q = format!(
            r#"UPDATE accounts SET name=$1, email=$2, status=$3, token=$4,
                proxy_url=$5, billing_mode=$6, concurrency=$7, priority=$8, updated_at={}
            WHERE id=$9"#,
            self.now_expr()
        );
        sqlx::query(&q)
            .bind(&a.name)
            .bind(&a.email)
            .bind(a.status.to_string())
            .bind(&a.token)
            .bind(&a.proxy_url)
            .bind(a.billing_mode.to_string())
            .bind(a.concurrency)
            .bind(a.priority)
            .bind(a.id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn update_status(
        &self,
        id: i64,
        status: AccountStatus,
    ) -> Result<(), AppError> {
        let q = format!(
            "UPDATE accounts SET status=$1, updated_at={} WHERE id=$2",
            self.now_expr()
        );
        sqlx::query(&q)
            .bind(status.to_string())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_rate_limit(
        &self,
        id: i64,
        reset_at: DateTime<Utc>,
    ) -> Result<(), AppError> {
        let q = format!(
            "UPDATE accounts SET rate_limited_at=$1, rate_limit_reset_at=$2, updated_at={} WHERE id=$3",
            self.now_expr()
        );
        sqlx::query(&q)
            .bind(self.fmt_time(Utc::now()))
            .bind(self.fmt_time(reset_at))
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn clear_rate_limit(&self, id: i64) -> Result<(), AppError> {
        let q = format!(
            "UPDATE accounts SET rate_limited_at=NULL, rate_limit_reset_at=NULL, updated_at={} WHERE id=$1",
            self.now_expr()
        );
        sqlx::query(&q)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete(&self, id: i64) -> Result<(), AppError> {
        sqlx::query("DELETE FROM accounts WHERE id=$1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_by_id(&self, id: i64) -> Result<Account, AppError> {
        let row: AnyRow = sqlx::query(
            &format!("SELECT {} FROM accounts WHERE id=$1", ACCOUNT_COLS),
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        Ok(Self::row_to_account(&row))
    }

    pub async fn list(&self) -> Result<Vec<Account>, AppError> {
        let rows: Vec<AnyRow> = sqlx::query(
            &format!(
                "SELECT {} FROM accounts ORDER BY priority ASC, id ASC",
                ACCOUNT_COLS
            ),
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(Self::row_to_account).collect())
    }

    pub async fn count(&self) -> Result<i64, AppError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);
        Ok(count)
    }

    pub async fn list_paged(&self, page: i64, page_size: i64) -> Result<Vec<Account>, AppError> {
        let offset = (page - 1) * page_size;
        let q = format!(
            "SELECT {} FROM accounts ORDER BY priority ASC, id ASC LIMIT $1 OFFSET $2",
            ACCOUNT_COLS
        );
        let rows: Vec<AnyRow> = sqlx::query(&q)
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(Self::row_to_account).collect())
    }

    pub async fn update_usage(&self, id: i64, usage_data: &str) -> Result<(), AppError> {
        let q = format!(
            "UPDATE accounts SET usage_data=$1, usage_fetched_at=$2, updated_at={} WHERE id=$3",
            self.now_expr()
        );
        sqlx::query(&q)
            .bind(usage_data)
            .bind(self.fmt_time(Utc::now()))
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_schedulable(&self) -> Result<Vec<Account>, AppError> {
        let q = format!(
            r#"SELECT {} FROM accounts
            WHERE status='active'
              AND (rate_limit_reset_at IS NULL OR rate_limit_reset_at < {})
            ORDER BY priority ASC, id ASC"#,
            ACCOUNT_COLS,
            self.now_expr()
        );
        let rows: Vec<AnyRow> = sqlx::query(&q).fetch_all(&self.pool).await?;
        Ok(rows.iter().map(Self::row_to_account).collect())
    }
}

const ACCOUNT_COLS: &str = r#"id, name, email, status, token, proxy_url, device_id,
    canonical_env, canonical_prompt_env, canonical_process,
    billing_mode, concurrency, priority, rate_limited_at, rate_limit_reset_at,
    usage_data, usage_fetched_at, created_at, updated_at"#;
