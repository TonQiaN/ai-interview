//! 错误类型。

use std::time::Duration;

/// 调用大模型时可能出的错。
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    /// 配置不对，例如没有设置密钥。
    #[error("大模型配置有误：{0}")]
    Config(String),

    /// 密钥无效（HTTP 401）。
    #[error("大模型密钥无效：{0}")]
    Auth(String),

    /// 账号余额不足（HTTP 402）。
    #[error("大模型账号余额不足：{0}")]
    InsufficientBalance(String),

    /// 被限速（HTTP 429），重试之后仍然失败。
    #[error("大模型请求被限速：{0}")]
    RateLimited(String),

    /// 服务端返回了其他错误状态。
    #[error("大模型接口返回 {status}：{message}")]
    Api { status: u16, message: String },

    /// 请求超时。
    #[error("大模型请求超时（{0:?}）")]
    Timeout(Duration),

    /// 网络层出错。
    #[error("连接大模型失败：{0}")]
    Transport(String),

    /// 响应的格式和预期不符。
    #[error("大模型响应无法解析：{0}")]
    Decode(String),

    /// 要求 JSON 输出，重试之后内容仍然是空的。
    #[error("大模型连续 {attempts} 次返回空内容")]
    EmptyContent { attempts: u32 },

    /// 要求 JSON 输出，重试之后仍然解析不成目标类型。
    #[error("大模型连续 {attempts} 次输出的 JSON 不符合要求：{message}")]
    InvalidJson { attempts: u32, message: String },

    /// 输出到了上限被截断。
    #[error("大模型输出被截断，max_tokens 不够")]
    Truncated,
}

impl LlmError {
    /// 这个错值不值得原样重试。
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::RateLimited(_) | Self::Timeout(_) | Self::Transport(_) => true,
            Self::Api { status, .. } => *status >= 500,
            _ => false,
        }
    }
}
