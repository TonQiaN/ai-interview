//! 这个 crate 的错误类型。

use interview_llm::LlmError;

/// 处理岗位、候选人和简历时可能出的错。
///
/// 报错信息会显示给 HR（比如写进 `resumes.parse_error`），所以用中文、说人话，
/// 并且不带简历正文。
#[derive(Debug, thiserror::Error)]
pub enum TalentError {
    /// 文件是空的。
    #[error("文件是空的")]
    EmptyFile,

    /// 文件类型不在支持范围里。
    #[error("不支持这种文件：{0}。可以上传 PDF、图片（PNG、JPG、WebP）或纯文本")]
    UnsupportedFile(String),

    /// 把 PDF 渲染成图片时出错。
    #[error("PDF 打不开：{0}")]
    Render(String),

    /// 没有从文件里读出文字。
    #[error("没有从文件里读出文字，可能是空白页或者图片太模糊")]
    Unreadable,

    /// 大模型判断这不是一份简历。
    #[error("这份文件看起来不是简历：{0}")]
    NotAResume(String),

    /// 大模型判断这不是一段岗位描述。
    #[error("这段文字看起来不是岗位描述：{0}")]
    NotAPosition(String),

    /// 配置不对。
    #[error("配置有误：{0}")]
    Config(String),

    /// 调用大模型失败。
    #[error(transparent)]
    Llm(#[from] LlmError),
}
