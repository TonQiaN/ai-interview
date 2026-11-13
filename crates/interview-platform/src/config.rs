//! 配置：全部从环境变量读。
//!
//! 每个 crate 自己定义配置结构和 `from_env()`，用这里的小工具读变量，
//! 缺必填项时错误里带变量名。变量名登记在仓库根的 `.env.example`。

use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;

/// 配置有问题。信息里只有变量名，不带变量的值（值可能是密钥）。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("缺少环境变量 {0}")]
    Missing(&'static str),
    #[error("环境变量 {name} 的值不合法：{reason}")]
    Invalid { name: &'static str, reason: String },
}

/// 读仓库根（或当前目录往上找到的第一个）`.env`。已经在环境里的变量不会被覆盖。
/// 没有 `.env` 不算错。
pub fn load_dotenv() {
    let _ = dotenvy::dotenv();
}

/// 读一个可选的变量。没设、或者只有空白，都算没有。
pub fn env_opt(name: &'static str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// 读一个必填的变量。
pub fn env_required(name: &'static str) -> Result<String, ConfigError> {
    env_opt(name).ok_or(ConfigError::Missing(name))
}

/// 读一个变量并解析；没设就用默认值。
pub fn env_parse<T>(name: &'static str, default: T) -> Result<T, ConfigError>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    match env_opt(name) {
        None => Ok(default),
        Some(raw) => raw.parse().map_err(|e: T::Err| ConfigError::Invalid {
            name,
            reason: e.to_string(),
        }),
    }
}

/// 读一个开关：`1`、`true`、`yes`、`on` 是开，`0`、`false`、`no`、`off` 是关。
pub fn env_flag(name: &'static str, default: bool) -> Result<bool, ConfigError> {
    match env_opt(name).map(|v| v.to_ascii_lowercase()).as_deref() {
        None => Ok(default),
        Some("1" | "true" | "yes" | "on") => Ok(true),
        Some("0" | "false" | "no" | "off") => Ok(false),
        Some(_) => Err(ConfigError::Invalid {
            name,
            reason: "要写 true 或 false".to_owned(),
        }),
    }
}

/// 平台自己的配置。
#[derive(Debug, Clone)]
pub struct PlatformConfig {
    /// `DATABASE_URL`
    pub database_url: String,
    /// `AI_INTERVIEW_BIND`，默认 `127.0.0.1:8080`
    pub bind: SocketAddr,
    /// `AI_INTERVIEW_PUBLIC_URL`，生成面试链接用的对外地址，不带结尾的 `/`
    pub public_url: String,
    /// `AI_INTERVIEW_DATA_DIR`，默认 `./data`。本地文件存储放在它下面的 `blobs/`
    pub data_dir: PathBuf,
    /// `AI_INTERVIEW_WEB_DIR`，默认 `./web/dist`。前端构建产物，下面有 `admin/` 和 `candidate/`
    pub web_dir: PathBuf,
    /// `AI_INTERVIEW_ADMIN_EMAIL`、`AI_INTERVIEW_ADMIN_PASSWORD`：库里没有账号时用它们建第一个管理员
    pub bootstrap_admin: Option<BootstrapAdmin>,
    /// `AI_INTERVIEW_TASK_WORKERS`，同时执行几个后台任务，默认 4；设成 0 就不在这个进程里执行任务
    pub task_workers: usize,
}

/// 第一个管理员账号。
#[derive(Clone)]
pub struct BootstrapAdmin {
    pub email: String,
    pub password: String,
}

impl std::fmt::Debug for BootstrapAdmin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BootstrapAdmin")
            .field("email", &self.email)
            .field("password", &"***")
            .finish()
    }
}

impl PlatformConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let bootstrap_admin = match (
            env_opt("AI_INTERVIEW_ADMIN_EMAIL"),
            env_opt("AI_INTERVIEW_ADMIN_PASSWORD"),
        ) {
            (Some(email), Some(password)) => Some(BootstrapAdmin { email, password }),
            (None, None) => None,
            (Some(_), None) => return Err(ConfigError::Missing("AI_INTERVIEW_ADMIN_PASSWORD")),
            (None, Some(_)) => return Err(ConfigError::Missing("AI_INTERVIEW_ADMIN_EMAIL")),
        };
        Ok(Self {
            database_url: env_required("DATABASE_URL")?,
            bind: env_parse(
                "AI_INTERVIEW_BIND",
                SocketAddr::from(([127, 0, 0, 1], 8080)),
            )?,
            public_url: env_opt("AI_INTERVIEW_PUBLIC_URL")
                .unwrap_or_else(|| "http://localhost:8080".to_owned())
                .trim_end_matches('/')
                .to_owned(),
            data_dir: env_parse("AI_INTERVIEW_DATA_DIR", PathBuf::from("./data"))?,
            web_dir: env_parse("AI_INTERVIEW_WEB_DIR", PathBuf::from("./web/dist"))?,
            bootstrap_admin,
            task_workers: env_parse("AI_INTERVIEW_TASK_WORKERS", 4)?,
        })
    }

    /// 测试用的配置：不读环境变量。
    pub fn for_tests() -> Self {
        Self {
            database_url: String::new(),
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            public_url: "http://localhost:8080".to_owned(),
            data_dir: PathBuf::from("./data"),
            web_dir: PathBuf::from("./web/dist"),
            bootstrap_admin: None,
            task_workers: 0,
        }
    }

    /// 对外地址是 https 时，cookie 要带 Secure。
    pub fn cookies_secure(&self) -> bool {
        self.public_url.starts_with("https://")
    }

    /// 候选人打开的面试链接。令牌放在 `#` 后面，不会进服务器的访问日志。
    pub fn invite_url(&self, token: &str) -> String {
        format!("{}/i/#{token}", self.public_url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_url_keeps_the_token_out_of_the_path() {
        let config = PlatformConfig::for_tests();
        assert_eq!(config.invite_url("abc"), "http://localhost:8080/i/#abc");
    }

    #[test]
    fn bootstrap_admin_debug_hides_the_password() {
        let admin = BootstrapAdmin {
            email: "hr@example.com".into(),
            password: "secret".into(),
        };
        let shown = format!("{admin:?}");
        assert!(!shown.contains("secret"));
    }
}
