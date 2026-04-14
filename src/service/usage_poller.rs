use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, info, warn};

use crate::model::account::{AccountAuthType, AccountStatus};
use crate::service::account::AccountService;

/// 账户间隔,避免瞬间打爆 Anthropic usage 端点。
const PER_ACCOUNT_GAP: Duration = Duration::from_millis(500);

/// 后台定时拉取所有 OAuth 账户用量数据的服务。
pub struct UsagePollerService {
    account_svc: Arc<AccountService>,
    interval: Duration,
}

impl UsagePollerService {
    pub fn new(account_svc: Arc<AccountService>, interval: Duration) -> Self {
        Self {
            account_svc,
            interval,
        }
    }

    /// 启动后台循环。应在 main 中 tokio::spawn 调用。
    pub async fn run(self: Arc<Self>) {
        info!("usage poller: started, interval = {:?}", self.interval);
        // 启动后先等一个 interval,避免和应用冷启动抢资源
        sleep(self.interval).await;
        loop {
            self.tick().await;
            sleep(self.interval).await;
        }
    }

    async fn tick(&self) {
        let accounts = match self.account_svc.list_accounts().await {
            Ok(list) => list,
            Err(e) => {
                warn!("usage poller: list accounts failed: {}", e);
                return;
            }
        };

        let targets: Vec<i64> = accounts
            .iter()
            .filter(|a| a.auth_type == AccountAuthType::Oauth && a.status == AccountStatus::Active)
            .map(|a| a.id)
            .collect();

        debug!("usage poller: polling {} oauth accounts", targets.len());

        for id in targets {
            match self.account_svc.refresh_usage(id).await {
                Ok(_) => debug!("usage poller: refreshed account {}", id),
                Err(e) => warn!("usage poller: account {} failed: {}", id, e),
            }
            sleep(PER_ACCOUNT_GAP).await;
        }
    }
}
