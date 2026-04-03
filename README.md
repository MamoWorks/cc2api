# Claude Code Gateway (Rust)

反检测网关 + 号池管理平台（Rust 实现）。

## 功能

- **号池管理**：多 Claude 账号轮转，自动生成设备指纹，每号可配独立代理
- **L5 TLS 指纹**：使用 craftls 精确复现 Node.js ClientHello（JA3/JA4 验证通过）
- **反检测**：Header wire casing、系统提示词改写、硬件指纹伪装、遥测清洗
- **智能路由**：sticky session (24h)、优先级选号、并发控制、自动限速处理
- **双模式**：Claude Code 客户端（替换模式）+ 直接 API 调用（注入模式）
- **Billing 控制**：每账号独立选择清除或 CCH hash 重写 billing header
- **Web 管理**：Vue 3 管理面板，账号 CRUD、连通性测试
- **双数据库**：SQLite（默认）/ PostgreSQL
- **缓存**：Redis（可选）/ 内存

## 快速开始

### 本地开发

```bash
# 1. 复制环境变量
cp .env.example .env

# 2. 构建前端 + 启动后端
./scripts/dev.sh      # Linux/macOS
scripts\dev.bat        # Windows
```

### 生产构建

```bash
# 当前平台
./scripts/build.sh                # Linux/macOS
scripts\build.bat                  # Windows

# 交叉编译
./scripts/build.sh linux-amd64    # Linux x86_64
./scripts/build.sh linux-arm64    # Linux ARM64
scripts\build.bat win-amd64       # Windows x86_64
scripts\build.bat linux-amd64     # Windows → Linux x86_64
scripts\build.bat linux-arm64     # Windows → Linux ARM64
```

构建产物输出到 `dist/` 目录。

### Docker 部署

```bash
cd docker
docker compose up -d
```

### 手动构建

```bash
# 构建前端
cd web && npm ci && npm run build && cd ..

# 构建后端
cargo build --release

# 运行
./target/release/cc2api
```

## 配置

所有配置通过环境变量或 `.env` 文件：


| 变量                | 默认值              | 说明                           |
| ----------------- | ---------------- | ---------------------------- |
| `SERVER_HOST`     | `0.0.0.0`        | 监听地址                         |
| `SERVER_PORT`     | `5674`           | 监听端口                         |
| `TLS_CERT_FILE`   | -                | TLS 证书路径（留空不启用 HTTPS）        |
| `TLS_KEY_FILE`    | -                | TLS 私钥路径                     |
| `DATABASE_DRIVER` | `sqlite`         | 数据库驱动（`sqlite` / `postgres`） |
| `DATABASE_DSN`    | `data/cc2api.db` | 连接串                          |
| `REDIS_HOST`      | -                | Redis 地址（留空使用内存缓存）           |
| `ADMIN_PASSWORD`  | `admin`          | 管理面板密码                       |
| `ADMIN_API_KEY`   | `cc2api-key`     | 网关 API Key                   |
| `LOG_LEVEL`       | `info`           | 日志级别                         |


## API

### 网关（API Key 认证）


| 方法    | 路径           | 说明            |
| ----- | ------------ | ------------- |
| `*`   | `/v1/*`      | Claude API 代理 |
| `*`   | `/api/*`     | Claude API 代理 |
| `GET` | `/v1/models` | 模型列表          |


### 管理（密码认证）


| 方法       | 路径                         | 说明       |
| -------- | -------------------------- | -------- |
| `GET`    | `/admin/accounts`          | 账号列表     |
| `POST`   | `/admin/accounts`          | 创建账号     |
| `PUT`    | `/admin/accounts/:id`      | 更新账号     |
| `DELETE` | `/admin/accounts/:id`      | 删除账号     |
| `POST`   | `/admin/accounts/:id/test` | 测试 Token |
| `GET`    | `/admin/dashboard`         | 仪表盘      |
| `GET`    | `/_health`                 | 健康检查     |


### 创建账号

```bash
curl -X POST http://localhost:5674/admin/accounts \
  -H "Authorization: Bearer admin" \
  -H "Content-Type: application/json" \
  -d '{
    "email": "user@example.com",
    "token": "sk-ant-...",
    "name": "account-1",
    "billing_mode": "strip"
  }'
```

`billing_mode`：`strip`（默认，清除 billing header）或 `rewrite`（CCH hash 重写）。

## 项目结构

```
rust/
├── docker/             # Docker 构建文件
│   ├── Dockerfile
│   └── docker-compose.yml
├── src/
│   ├── main.rs         # 入口
│   ├── config.rs       # 环境变量配置
│   ├── error.rs        # 错误处理
│   ├── model/          # 数据模型（Account, Usage, Identity）
│   ├── store/          # 存储层（DB, AccountStore, Cache, Memory, Redis）
│   ├── service/        # 业务逻辑（Gateway, Rewriter, Account, Usage）
│   ├── handler/        # HTTP 路由与处理器
│   ├── middleware/      # 认证中间件
│   └── tlsfp/          # TLS 指纹（craftls, Node.js ClientHello）
├── web/                # Vue 3 管理面板
├── scripts/            # 构建与开发脚本
│   ├── dev.sh / dev.bat
│   └── build.sh / build.bat
├── .env.example        # 环境变量示例
└── Cargo.toml          # Rust 项目配置
```

## CI/CD

推送 `v*` 标签自动触发 GitHub Actions：

```bash
# 1. 更新 .version 中的版本号
# 2. 打标签发布
git tag v1.0.0
git push origin v1.0.0
```

自动执行：

- **构建二进制**: linux-amd64、linux-arm64、win-amd64
- **Docker 镜像**: 多架构（amd64 + arm64），推送至 GHCR
- **GitHub Release**: 附带所有平台构建产物

版本和镜像名由 `.version` 文件控制。

## TLS 指纹验证

通过 [tls.peet.ws](https://tls.peet.ws) 验证，Rust 版（craftls）与 Node.js、Go（uTLS）三方指纹完全一致：

```
JA3 Hash:  d67b094811e5145139d7cea5f014309f  (三方一致)
JA4:       t13d5212h1_b262b3658495_8e6e362c5eac  (三方一致)
密码套件:   52 个
扩展:       12 个
曲线:       8 个（含后量子 X25519MLKEM768）
签名算法:   26 个（含 ML-DSA）
```

