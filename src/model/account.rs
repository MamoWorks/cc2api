use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AccountStatus {
    Active,
    Error,
    Disabled,
}

impl Default for AccountStatus {
    fn default() -> Self {
        Self::Active
    }
}

impl std::fmt::Display for AccountStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Error => write!(f, "error"),
            Self::Disabled => write!(f, "disabled"),
        }
    }
}

impl From<String> for AccountStatus {
    fn from(s: String) -> Self {
        match s.as_str() {
            "active" => Self::Active,
            "error" => Self::Error,
            "disabled" => Self::Disabled,
            _ => Self::Active,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BillingMode {
    Strip,
    Rewrite,
}

impl Default for BillingMode {
    fn default() -> Self {
        Self::Strip
    }
}

impl std::fmt::Display for BillingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Strip => write!(f, "strip"),
            Self::Rewrite => write!(f, "rewrite"),
        }
    }
}

impl From<String> for BillingMode {
    fn from(s: String) -> Self {
        match s.as_str() {
            "rewrite" => Self::Rewrite,
            _ => Self::Strip,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub status: AccountStatus,
    pub token: String,
    #[serde(default)]
    pub proxy_url: String,
    pub device_id: String,
    pub canonical_env: Value,
    #[serde(rename = "canonical_prompt_env")]
    pub canonical_prompt: Value,
    pub canonical_process: Value,
    pub billing_mode: BillingMode,
    #[serde(default = "default_concurrency")]
    pub concurrency: i32,
    #[serde(default = "default_priority")]
    pub priority: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limited_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_reset_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub usage_data: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_fetched_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn default_concurrency() -> i32 { 3 }
fn default_priority() -> i32 { 50 }

impl Account {
    pub fn is_schedulable(&self) -> bool {
        if self.status != AccountStatus::Active {
            return false;
        }
        if let Some(reset) = self.rate_limit_reset_at {
            if Utc::now() < reset {
                return false;
            }
        }
        true
    }
}

/// 存储 20+ 维度的环境指纹数据。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CanonicalEnvData {
    pub platform: String,
    pub platform_raw: String,
    pub arch: String,
    pub node_version: String,
    pub terminal: String,
    pub package_managers: String,
    pub runtimes: String,
    #[serde(default)]
    pub is_running_with_bun: bool,
    #[serde(default)]
    pub is_ci: bool,
    #[serde(default)]
    pub is_claubbit: bool,
    #[serde(default)]
    pub is_claude_code_remote: bool,
    #[serde(default)]
    pub is_local_agent_mode: bool,
    #[serde(default)]
    pub is_conductor: bool,
    #[serde(default)]
    pub is_github_action: bool,
    #[serde(default)]
    pub is_claude_code_action: bool,
    #[serde(default)]
    pub is_claude_ai_auth: bool,
    pub version: String,
    pub version_base: String,
    pub build_time: String,
    pub deployment_environment: String,
    pub vcs: String,
}

/// 系统提示词中的环境改写数据。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CanonicalPromptEnvData {
    pub platform: String,
    pub shell: String,
    pub os_version: String,
    pub working_dir: String,
}

/// 硬件指纹配置。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CanonicalProcessData {
    pub constrained_memory: i64,
    pub rss_range: [i64; 2],
    pub heap_total_range: [i64; 2],
    pub heap_used_range: [i64; 2],
}
