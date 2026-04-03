use chrono::{DateTime, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiToken {
    pub id: i64,
    pub name: String,
    pub token: String,
    /// 逗号分隔的可用账号 ID（空字符串表示不限制）
    pub allowed_accounts: String,
    /// 逗号分隔的不可用账号 ID（空字符串表示不限制）
    pub blocked_accounts: String,
    pub status: ApiTokenStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiTokenStatus {
    Active,
    Disabled,
}

impl std::fmt::Display for ApiTokenStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Disabled => write!(f, "disabled"),
        }
    }
}

impl From<String> for ApiTokenStatus {
    fn from(s: String) -> Self {
        match s.as_str() {
            "active" => Self::Active,
            _ => Self::Disabled,
        }
    }
}

impl ApiToken {
    /// 解析可用账号 ID 列表
    pub fn allowed_account_ids(&self) -> Vec<i64> {
        parse_id_list(&self.allowed_accounts)
    }

    /// 解析不可用账号 ID 列表
    pub fn blocked_account_ids(&self) -> Vec<i64> {
        parse_id_list(&self.blocked_accounts)
    }
}

fn parse_id_list(s: &str) -> Vec<i64> {
    if s.is_empty() {
        return vec![];
    }
    s.split(',')
        .filter_map(|id| id.trim().parse::<i64>().ok())
        .collect()
}

/// 生成 sk- 开头的 64 位令牌（sk- + 61 位随机字符）
pub fn generate_token() -> String {
    const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut rng = rand::thread_rng();
    let random: String = (0..61)
        .map(|_| CHARSET[rng.gen_range(0..CHARSET.len())] as char)
        .collect();
    format!("sk-{}", random)
}
