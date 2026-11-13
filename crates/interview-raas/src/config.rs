use std::fmt;
use std::time::Duration;

use crate::error::RaasError;

/// RAAS 接口地址，例如 `https://raas.example.com/raas-api`。带不带末尾的 `/api/v1` 都可以。
pub const BASE_URL_ENV_VAR: &str = "RAAS_BASE_URL";
/// RAAS 登录账号。
pub const USERNAME_ENV_VAR: &str = "RAAS_ADMIN_USERNAME";
/// RAAS 登录口令。
pub const PASSWORD_ENV_VAR: &str = "RAAS_ADMIN_PASSWORD";

/// 连接 RAAS 需要的配置。口令不会出现在 `Debug` 输出里。
#[derive(Clone)]
pub struct RaasConfig {
    pub(crate) base_url: String,
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) timeout: Duration,
}

impl RaasConfig {
    /// 普通请求的默认超时。下载原始简历文件用的是它的两倍。
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

    pub fn new(
        base_url: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            username: username.into(),
            password: password.into(),
            timeout: Self::DEFAULT_TIMEOUT,
        }
    }

    /// 从环境变量读配置：[`BASE_URL_ENV_VAR`]、[`USERNAME_ENV_VAR`]、[`PASSWORD_ENV_VAR`]。
    pub fn from_env() -> Result<Self, RaasError> {
        Ok(Self::new(
            required_env(BASE_URL_ENV_VAR)?,
            required_env(USERNAME_ENV_VAR)?,
            required_env(PASSWORD_ENV_VAR)?,
        ))
    }

    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// 接口根地址，统一成以 `/api/v1` 结尾、不带末尾斜杠的形式。
    pub(crate) fn api_root(&self) -> String {
        let trimmed = self.base_url.trim().trim_end_matches('/');
        if trimmed.ends_with("/api/v1") {
            trimmed.to_owned()
        } else {
            format!("{trimmed}/api/v1")
        }
    }
}

impl fmt::Debug for RaasConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RaasConfig")
            .field("base_url", &self.base_url)
            .field("username", &"<已隐藏>")
            .field("password", &"<已隐藏>")
            .field("timeout", &self.timeout)
            .finish()
    }
}

fn required_env(name: &'static str) -> Result<String, RaasError> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(RaasError::MissingConfig(name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_root_accepts_both_forms() {
        for base in [
            "https://raas.example.com/raas-api",
            "https://raas.example.com/raas-api/",
            "https://raas.example.com/raas-api/api/v1",
            "https://raas.example.com/raas-api/api/v1/",
        ] {
            let config = RaasConfig::new(base, "u", "p");
            assert_eq!(
                config.api_root(),
                "https://raas.example.com/raas-api/api/v1"
            );
        }
    }

    #[test]
    fn debug_hides_credentials() {
        let config = RaasConfig::new("https://raas.example.com", "admin-account", "s3cret-pass");
        let printed = format!("{config:?}");
        assert!(!printed.contains("admin-account"));
        assert!(!printed.contains("s3cret-pass"));
    }
}
