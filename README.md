# CC-Bridge

基于 Rust 实现的 CC 反检测网关与账号池管理平台。将网关转发、账号调度、令牌鉴权、用量管理和 Web 管理后台整合到单一二进制文件中。

## 核心能力

- **多账号池**：维护多个 Claude 账号，支持 Setup Token 和 OAuth 双认证模式
- **令牌鉴权**：通过 API Token 对调用方鉴权，不暴露真实账号凭证
- **粘性会话**：同一会话 24h 内命中同一账号，降低行为漂移
- **优先级调度**：按 `priority` 升序选号，同优先级随机
- **并发控制**：每账号独立并发上限，支持 Redis 或内存计数
- **自动限速回避**：429 → 暂停 5h；403 → 永久停用（限流期内的 403 跳过，避免误判）
- **手动启停**：管理后台一键启用/停用账号
- **请求反检测改写**：改写 UA、系统提示、环境指纹、遥测字段
- **AI Gateway 指纹过滤**：过滤 LiteLLM / Helicone / Portkey / Cloudflare AI Gateway / Kong / BrainTrust 响应头
- **TLS 指纹伪装**：自定义 `craftls` 复现 Node.js 风格 ClientHello
- **OAuth 授权登录**：内置 PKCE 流程，一键授权获取凭证
- **自动遥测**：网关代发遥测请求，10min TTL 自动续期
- **管理后台**：Vue 3 Web 界面，账号/令牌增删改查、连接测试、用量刷新、仪表盘
- **多存储后端**：SQLite / PostgreSQL + Redis / 内存缓存

## 快速开始

### 环境要求

- Rust ≥ 1.82、Node.js 22、npm
- 可选：Redis、PostgreSQL、Docker、Zig + `cargo-zigbuild`（交叉编译）

### 启动

```bash
cp .env.example .env
./scripts/dev.sh          # Linux / macOS
# scripts\dev.bat         # Windows
```

启动后：

| 入口 | 地址 |
| --- | --- |
| 管理后台 | `http://127.0.0.1:5674/` |
| 登录页 | `http://127.0.0.1:5674/login` |
| 网关 | 除前端页面、`/assets/*`、`/admin/*` 外的所有路径 |

默认管理员密码：`admin`

### 基本使用

1. 登录管理后台
2. 新建账号（手动填写或点击"授权登录"通过 OAuth 一键授权）
3. 建议同时填写 `account_uuid`、`organization_uuid`、`subscription_type`
4. 在"令牌"页面创建 API Token
5. 调用网关时使用 `x-api-key: sk-...` 或 `Authorization: Bearer sk-...`

### 调用示例

```bash
curl http://127.0.0.1:5674/v1/messages \
  -H "Authorization: Bearer sk-your-gateway-token" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-sonnet-4-6",
    "max_tokens": 128,
    "messages": [{"role": "user", "content": "hello"}]
  }'
```

## 配置说明

通过 `.env` 文件或环境变量配置，优先级：进程环境变量 > `.env` > 代码默认值。

### 服务端

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `SERVER_HOST` | `0.0.0.0` | 监听地址 |
| `SERVER_PORT` | `5674` | 监听端口 |
| `TLS_CERT_FILE` | 空 | 证书路径（当前未接入 TLS 监听，需反代） |
| `TLS_KEY_FILE` | 空 | 私钥路径 |
| `LOG_LEVEL` | `info` | `debug` / `info` / `warn` / `error` |
| `ADMIN_PASSWORD` | `admin` | 管理后台共享密码 |

### 数据库

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `DATABASE_DRIVER` | `sqlite` | `sqlite` 或 `postgres` |
| `DATABASE_DSN` | `data/claude-code-gateway.db` | 完整 DSN，设置后优先使用 |
| `DATABASE_HOST` | `localhost` | PostgreSQL 主机（DSN 为空时拼接） |
| `DATABASE_PORT` | `5432` | PostgreSQL 端口 |
| `DATABASE_USER` | `postgres` | PostgreSQL 用户名 |
| `DATABASE_PASSWORD` | 空 | PostgreSQL 密码 |
| `DATABASE_DBNAME` | `claude_code_gateway` | PostgreSQL 数据库名 |

SQLite 自动创建目录并启用 WAL 模式。PostgreSQL 无 DSN 时自动拼接连接串。

### Redis（可选）

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `REDIS_HOST` | 空 | 不设置则使用内存缓存 |
| `REDIS_PORT` | `6379` | 端口 |
| `REDIS_PASSWORD` | 空 | 密码 |
| `REDIS_DB` | `0` | 数据库编号 |

Redis 用于粘性会话和并发计数。单实例无需 Redis，多实例部署请启用。

### 最小配置

```env
SERVER_HOST=0.0.0.0
SERVER_PORT=5674
DATABASE_DRIVER=sqlite
DATABASE_DSN=data/claude-code-gateway.db
ADMIN_PASSWORD=change-me
LOG_LEVEL=info
```

## 构建与部署

### 开发

```bash
# 方式一：脚本启动（自动检测前端变更）
./scripts/dev.sh

# 方式二：前后端分离
cd web && npm ci && npm run dev   # 终端 A：前端 :3000
cargo run                          # 终端 B：后端 :5674
```

### 生产构建

```bash
# 当前平台
./scripts/build.sh

# 交叉编译
./scripts/build.sh linux-amd64
./scripts/build.sh linux-arm64

# 手动构建
cd web && npm ci && npm run build && cd ..
cargo build --release
./target/release/claude-code-gateway
```

### Docker

```bash
cp .env.example .env
cd docker && docker compose up -d
```

SQLite 数据持久化到命名卷 `claude-code-gateway-data`。

### 生产建议

- 使用 Nginx / Caddy 等反代做 TLS 终止
- 设置强随机 `ADMIN_PASSWORD`
- 多实例部署启用 Redis
- 对管理后台路径做网络隔离

## HTTP API

### 认证

- **管理 API**：`x-api-key: <ADMIN_PASSWORD>` 或 `Authorization: Bearer <ADMIN_PASSWORD>`
- **网关 API**：`x-api-key: <sk-...>` 或 `Authorization: Bearer <sk-...>`

### 管理接口

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `GET` | `/admin/dashboard` | 仪表盘统计 |
| `GET` | `/admin/accounts` | 账号列表（`page`/`page_size`） |
| `POST` | `/admin/accounts` | 创建账号 |
| `PUT` | `/admin/accounts/:id` | 更新账号 |
| `DELETE` | `/admin/accounts/:id` | 删除账号 |
| `POST` | `/admin/accounts/:id/test` | 测试账号 Token |
| `POST` | `/admin/accounts/:id/usage` | 刷新用量 |
| `GET` | `/admin/tokens` | 令牌列表 |
| `POST` | `/admin/tokens` | 创建令牌 |
| `PUT` | `/admin/tokens/:id` | 更新令牌 |
| `DELETE` | `/admin/tokens/:id` | 删除令牌 |
| `POST` | `/admin/oauth/generate-auth-url` | 生成 OAuth 授权链接 |
| `POST` | `/admin/oauth/generate-setup-token-url` | 生成 Setup Token 授权链接 |
| `POST` | `/admin/oauth/exchange-code` | 交换 OAuth 授权码 |
| `POST` | `/admin/oauth/exchange-setup-token-code` | 交换 Setup Token 授权码 |

### 网关接口

所有未命中前端页面、`/assets/*`、`/admin/*` 的请求进入网关 fallback，经 API Token 鉴权后转发到 `https://api.anthropic.com`。

### 保留路径

`/`、`/login`、`/tokens`、`/favicon.svg`、`/assets/*`、`/admin/*` 不会进入网关。

### 创建账号示例

```bash
# Setup Token 模式
curl -X POST http://127.0.0.1:5674/admin/accounts \
  -H "Authorization: Bearer admin" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "account-01",
    "email": "user@example.com",
    "auth_type": "setup_token",
    "setup_token": "sk-ant-xxxx",
    "proxy_url": "socks5://127.0.0.1:1080",
    "billing_mode": "strip",
    "account_uuid": "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx",
    "organization_uuid": "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx",
    "subscription_type": "pro",
    "concurrency": 3,
    "priority": 50,
    "auto_telemetry": false
  }'

# OAuth 模式
curl -X POST http://127.0.0.1:5674/admin/accounts \
  -H "Authorization: Bearer admin" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "account-02",
    "email": "user@example.com",
    "auth_type": "oauth",
    "access_token": "ant-oc_xxxx",
    "refresh_token": "ant-rt_xxxx",
    "expires_at": 1735689600000,
    "billing_mode": "rewrite",
    "account_uuid": "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx",
    "organization_uuid": "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx",
    "subscription_type": "max",
    "concurrency": 5,
    "priority": 10,
    "auto_telemetry": true
  }'
```

### 创建令牌示例

```bash
curl -X POST http://127.0.0.1:5674/admin/tokens \
  -H "Authorization: Bearer admin" \
  -H "Content-Type: application/json" \
  -d '{"name": "team-a", "allowed_accounts": "1,2", "blocked_accounts": ""}'
```

### 错误响应

统一格式 `{"error": "..."}`，常见状态码：400 / 401 / 404 / 429 / 502 / 503 / 500。

## OAuth 授权登录

管理后台内置 OAuth PKCE 授权流程：

1. 点击"授权登录"，选择模式：
   - **OAuth（完整权限）**：获取 access_token + refresh_token
   - **Setup Token（仅推理）**：获取 365 天有效的 access_token
2. 可选填写代理地址
3. 复制授权链接到浏览器完成登录
4. 从回调 URL 复制 `code`，粘贴到管理后台交换
5. 系统自动获取凭证和 `account_uuid`、`organization_uuid`、`email` 等信息
6. 点击"应用到新账号"自动填入表单

> 授权会话有效期 30 分钟。

## 自动遥测

开启 `auto_telemetry` 后，网关代替客户端发送遥测：

- **拦截**：客户端遥测请求返回 200，不转发上游
- **代发**：`/api/event_logging/batch`（每 10s）、`/api/eval/sdk-*`（每 6h）
- **触发**：账号收到 `/v1/messages` 请求时激活遥测会话（10min TTL，自动续期）
- **拦截路径**：`/api/event_logging/batch`、`/api/eval/*`、`/api/claude_code/metrics`、`/api/claude_code/organizations/metrics_enabled`

> Datadog 遥测由客户端直连 `browser-intake-datadoghq.com`，无法通过网关拦截。建议在网络层屏蔽。

## CI/CD

通过 `.version` 文件控制发布版本。GitHub Actions 工作流（`.github/workflows/release.yml`）：

- **自动触发**：推送到 `main` 且 `.version` 有变更
- **手动触发**：`workflow_dispatch`
- **产物**：Linux x86_64/arm64 + Windows x86_64 二进制、GHCR 多架构 Docker 镜像（`latest` / `<version>` / `v<version>`）

发布步骤：修改 `.version` 中的 `version` → 合入 `main` → 等待自动构建。

## 项目结构

```text
.
├── .github/workflows/       # GitHub Actions 发布流程
├── craftls/                 # 自定义 rustls 分支（TLS 指纹伪装）
├── docker/                  # Dockerfile 与 docker-compose.yml
├── scripts/                 # 开发与构建脚本
├── src/
│   ├── main.rs              # 程序入口
│   ├── config.rs            # 环境变量加载
│   ├── error.rs             # 统一错误类型
│   ├── handler/             # 路由与 HTTP handler
│   ├── middleware/          # 鉴权中间件
│   ├── model/               # Account / ApiToken / Identity 模型
│   ├── service/             # Gateway / Account / OAuth / Telemetry / Rewriter
│   ├── store/               # 数据库与缓存访问层
│   └── tlsfp/               # TLS 指纹客户端
├── web/                     # Vue 3 前端
│   ├── src/components/      # 页面组件
│   ├── src/api.ts           # API 封装
│   └── vite.config.ts       # Vite 配置
├── .env.example             # 配置模板
├── .version                 # 发布版本与镜像名
└── Cargo.toml               # Rust 项目清单
```

## 限制与注意事项

1. **TLS 未接入 HTTPS 监听**：需使用 Nginx / Caddy / Traefik 等反代做 TLS 终止
2. **无显式 `/_health` 和 `/v1/models`**：这些路径会进入网关 fallback 转发到上游
3. **Token 明文存储**：账号凭证和 API Token 以明文存储在数据库中，请保护数据库访问
4. **单共享密码**：无多用户/权限系统，建议强密码 + 可信网络 + 反代访问控制
5. **多实例需 Redis**：否则会话粘性和并发计数无法跨实例共享
6. **版本号硬编码**：identity 模块中的 Claude Code 版本号（当前 `2.1.81`）和构建时间为静态值，上游更新后需手动同步
7. **Datadog 遥测无法拦截**：客户端直连发送，建议网络层屏蔽

---

<details>
<summary><strong>网关内部工作机制</strong>（点击展开）</summary>

### 请求鉴权

网关请求经令牌鉴权中间件，令牌必须在 `api_tokens` 表中且状态为 `active`。

### 客户端类型识别

- `User-Agent` 以 `claude-code/` 或 `claude-cli/` 开头 → Claude Code 模式
- 请求体 `metadata.user_id` 存在 → Claude Code 模式
- 其余 → 纯 API 模式

### 会话哈希

- Claude Code：从 `metadata.user_id` 解析 `session_id`
- 纯 API：`sha256(UA + system/首条消息 + 小时窗口)`

### 账号过滤

每个 API Token 可配置 `allowed_accounts` 和 `blocked_accounts`（逗号分隔 ID）。

### 账号选择

1. 粘性绑定命中且可调度 → 复用
2. 否则从可调度账号（active + 未限流 + 未排除）中按 `priority` 升序选最优组
3. 同优先级随机选择 → 写入 24h 粘性绑定

### 并发控制

每账号 `concurrency` 上限，请求命中后抢占槽位，失败返回 429。槽位请求结束后自动释放。

### 限速与停用

| 上游状态码 | 行为 | 持续时间 |
| --- | --- | --- |
| `429` | 暂停调度（状态保持 active） | 5 小时自动恢复 |
| `403` | 永久停用（标记 disabled） | 手动启用 |

429 限流期内的 403 不会触发永久停用。

### 请求头改写

- User-Agent → `claude-code/<version> (external, cli)`
- 注入/合并 `anthropic-beta`、固定 `anthropic-version`
- 强制使用账号真实 `Authorization`
- 追加 `beta=true` 查询参数
- 还原 header wire casing

### 请求体改写

| 路径 | 改写内容 |
| --- | --- |
| `/v1/messages` | 系统提示词注入、`metadata.user_id`、环境/进程指纹、`cache_control`、billing 处理 |
| `/api/event_logging/batch` | `device_id`、`email`、`account_uuid`、`organization_uuid`、env/process 指纹、`user_attributes` JSON |
| `/api/eval/{clientKey}` | `id`、`deviceID`、`email`、`accountUUID`、`organizationUUID`、`subscriptionType`、移除 `apiBaseUrlHost` |
| 其他路径 | 通用身份字段改写 |

### TLS 指纹

所有上游请求通过 `craftls` 发出，模拟 Node.js TLS 指纹。每账号可配代理（HTTP / SOCKS5）。

### AI Gateway 指纹过滤

过滤响应头前缀：`x-litellm-`、`helicone-`、`x-portkey-`、`cf-aig-`、`x-kong-`、`x-bt-`。

</details>

<details>
<summary><strong>数据库表结构</strong>（点击展开）</summary>

### `accounts` 表

| 字段 | 说明 |
| --- | --- |
| `id` | 主键 |
| `name` / `email` | 账号标识（email 检查重复） |
| `status` | `active` / `error` / `disabled` |
| `auth_type` | `setup_token` / `oauth` |
| `token` | Setup Token |
| `access_token` / `refresh_token` / `oauth_expires_at` / `oauth_refreshed_at` | OAuth 凭证 |
| `auth_error` | 认证错误信息 |
| `proxy_url` | 账号专用代理 |
| `device_id` | 自动生成的设备 ID |
| `canonical_env` / `canonical_prompt_env` / `canonical_process` | 指纹 JSON |
| `billing_mode` | `strip` / `rewrite` |
| `account_uuid` / `organization_uuid` / `subscription_type` | 遥测改写用 |
| `concurrency` / `priority` | 调度参数 |
| `rate_limited_at` / `rate_limit_reset_at` / `disable_reason` | 限流/停用状态 |
| `usage_data` / `usage_fetched_at` | 用量缓存 |
| `auto_telemetry` / `telemetry_count` | 自动遥测 |

### `api_tokens` 表

| 字段 | 说明 |
| --- | --- |
| `id` | 主键 |
| `name` | 令牌名称 |
| `token` | 自动生成的 `sk-...` 令牌 |
| `allowed_accounts` / `blocked_accounts` | 账号 ID 列表（逗号分隔） |
| `status` | `active` / `disabled` |

服务启动时自动执行内建 SQL 迁移，不依赖外部 migration 文件。

</details>

<details>
<summary><strong>账号字段参考</strong>（点击展开）</summary>

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `email` | 是 | 账号邮箱 |
| `auth_type` | 否 | `setup_token`（默认）或 `oauth` |
| `setup_token` / `token` | 条件 | Setup Token 模式必填 |
| `access_token` / `refresh_token` | 条件 | OAuth 模式必填 |
| `expires_at` | 否 | OAuth access_token 过期时间（ms 时间戳） |
| `name` | 否 | 显示名称 |
| `proxy_url` | 否 | 专用代理 |
| `billing_mode` | 否 | `strip` 或 `rewrite` |
| `account_uuid` | 否 | 推荐填写，用于遥测改写 |
| `organization_uuid` | 否 | 推荐填写，用于遥测改写 |
| `subscription_type` | 否 | `max` / `pro` / `team` / `enterprise`，推荐填写 |
| `concurrency` | 否 | 最大并发，默认 3 |
| `priority` | 否 | 数值越小优先级越高，默认 50 |
| `auto_telemetry` | 否 | 是否开启自动遥测，默认 false |

创建时系统自动生成 `device_id`、`canonical_env`、`canonical_prompt_env`、`canonical_process`。

</details>

## 许可与依赖说明

项目包含自定义 `craftls` 目录。发布时请检查该目录下的许可证文件。
