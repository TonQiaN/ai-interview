use thiserror::Error;

/// 访问 RAAS 时可能出的错。
///
/// 报错信息里不带响应正文和查询参数，因为它们可能含候选人的个人信息。
#[derive(Debug, Error)]
pub enum RaasError {
    #[error("缺少环境变量 {0}")]
    MissingConfig(&'static str),

    #[error("RAAS 地址不合法：{0}")]
    InvalidBaseUrl(String),

    /// RAAS 拒绝了登录。连续登录失败会锁账号，所以之后这个客户端不会再尝试登录。
    #[error(
        "RAAS 拒绝登录（HTTP {status}）。为避免账号被锁，本客户端不会再重试登录，请先核对账号和口令"
    )]
    LoginRejected { status: u16 },

    /// 之前登录被拒过，这次直接放弃，没有发请求。
    #[error("之前登录 RAAS 被拒，已停止重试。请核对账号和口令后重新创建客户端")]
    LoginDisabled,

    #[error("RAAS 登录成功但没有返回会话令牌")]
    LoginWithoutToken,

    /// RAAS 返回了非成功状态码。`code` 是 RAAS 自己的错误码（如 `NOT_FOUND`）。
    #[error("RAAS 返回 HTTP {status}{}（{endpoint}）", .code.as_deref().map(|c| format!(" {c}")).unwrap_or_default())]
    Http {
        status: u16,
        code: Option<String>,
        endpoint: String,
    },

    /// 响应不是预期的 JSON 结构。只给出位置，不给出内容。
    #[error("RAAS 的响应解析失败（{endpoint}，第 {line} 行第 {column} 列，{kind}）")]
    Decode {
        endpoint: String,
        line: usize,
        column: usize,
        kind: &'static str,
    },

    #[error("连接 RAAS 失败：{0}")]
    Transport(#[source] reqwest::Error),
}

impl RaasError {
    /// 网络层错误统一从这里进：去掉地址，免得查询参数（可能是人名、手机号）进日志。
    pub(crate) fn transport(error: reqwest::Error) -> Self {
        Self::Transport(error.without_url())
    }

    pub(crate) fn decode(endpoint: &str, error: &serde_json::Error) -> Self {
        use serde_json::error::Category;
        Self::Decode {
            endpoint: endpoint.to_owned(),
            line: error.line(),
            column: error.column(),
            kind: match error.classify() {
                Category::Io => "读取中断",
                Category::Syntax => "不是合法的 JSON",
                Category::Data => "字段类型和预期不符",
                Category::Eof => "内容不完整",
            },
        }
    }

    /// 是不是「没找到」。
    #[must_use]
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::Http { status: 404, .. })
    }
}
