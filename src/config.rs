use std::env;

#[derive(Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub redis: Option<RedisConfig>,
    pub admin: AdminConfig,
    pub log_level: String,
}

#[derive(Clone)]
pub struct ServerConfig {
    pub port: u16,
    pub host: String,
    pub tls_cert: Option<String>,
    pub tls_key: Option<String>,
}

#[derive(Clone)]
pub struct DatabaseConfig {
    pub driver: Option<String>,
    pub dsn: Option<String>,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub dbname: String,
}

#[derive(Clone)]
pub struct RedisConfig {
    pub host: String,
    pub port: u16,
    pub password: String,
    pub db: i64,
}

#[derive(Clone)]
pub struct AdminConfig {
    pub password: String,
}

impl DatabaseConfig {
    pub fn driver(&self) -> String {
        self.driver.clone().unwrap_or_else(|| "sqlite".into())
    }

    pub fn dsn(&self) -> String {
        if let Some(dsn) = &self.dsn {
            return dsn.clone();
        }
        if self.driver() == "sqlite" {
            return "data/claude-code-gateway.db".into();
        }
        format!(
            "postgres://{}:{}@{}:{}/{}?sslmode=disable",
            self.user, self.password, self.host, self.port, self.dbname
        )
    }
}

impl Config {
    pub fn load() -> Self {
        dotenvy::dotenv().ok();

        let redis = env::var("REDIS_HOST").ok().map(|host| RedisConfig {
            host,
            port: env::var("REDIS_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(6379),
            password: env::var("REDIS_PASSWORD").unwrap_or_default(),
            db: env::var("REDIS_DB")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
        });

        Config {
            server: ServerConfig {
                port: env::var("SERVER_PORT")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(5674),
                host: env::var("SERVER_HOST").unwrap_or_else(|_| "0.0.0.0".into()),
                tls_cert: env::var("TLS_CERT_FILE").ok(),
                tls_key: env::var("TLS_KEY_FILE").ok(),
            },
            database: DatabaseConfig {
                driver: env::var("DATABASE_DRIVER").ok(),
                dsn: env::var("DATABASE_DSN").ok(),
                host: env::var("DATABASE_HOST").unwrap_or_else(|_| "localhost".into()),
                port: env::var("DATABASE_PORT")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(5432),
                user: env::var("DATABASE_USER").unwrap_or_else(|_| "postgres".into()),
                password: env::var("DATABASE_PASSWORD").unwrap_or_default(),
                dbname: env::var("DATABASE_DBNAME").unwrap_or_else(|_| "claude_code_gateway".into()),
            },
            redis,
            admin: AdminConfig {
                password: env::var("ADMIN_PASSWORD").unwrap_or_else(|_| "admin".into()),
            },
            log_level: env::var("LOG_LEVEL").unwrap_or_else(|_| "info".into()),
        }
    }
}
