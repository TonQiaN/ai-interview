//! HTTP 接口。路径和约定见 `docs/architecture/api.md`。

use axum::http::StatusCode;
use interview_llm::LlmError;
use interview_platform::ApiError;
use utoipa_axum::router::OpenApiRouter;

use crate::deps::Deps;
use crate::error::TalentError;

pub mod positions;

/// 岗位和候选人这条线的全部接口。
pub fn router(deps: Deps) -> OpenApiRouter {
    OpenApiRouter::new()
        .merge(positions::router())
        .with_state(deps)
}

/// 把这条线的错误换成接口的错误。用户能改正的是 422，大模型出问题是 503。
pub(crate) fn talent_error(error: TalentError) -> ApiError {
    let unprocessable = |code: &'static str, error: &TalentError| {
        ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, code, error.to_string())
    };
    match error {
        TalentError::EmptyFile => unprocessable("empty_file", &error),
        TalentError::UnsupportedFile(_) => unprocessable("unsupported_file", &error),
        TalentError::Render(_) | TalentError::Unreadable => {
            unprocessable("unreadable_file", &error)
        }
        TalentError::NotAResume(_) => unprocessable("not_a_resume", &error),
        TalentError::NotAPosition(_) => unprocessable("not_a_position", &error),
        TalentError::Config(_) => ApiError::internal(error),
        TalentError::Llm(error) => llm_error(error),
    }
}

fn llm_error(error: LlmError) -> ApiError {
    let message = match &error {
        LlmError::Config(_) | LlmError::Auth(_) => "大模型的配置有问题，请联系管理员",
        LlmError::InsufficientBalance(_) => "大模型账号余额不足，请联系管理员",
        LlmError::Truncated => "内容太长，大模型没能处理完，请缩短后再试",
        _ => "大模型暂时不可用，请稍后再试",
    };
    ApiError::unavailable(message)
        .with_code("llm_unavailable")
        .with_source(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_mistakes_are_422_and_model_trouble_is_503() {
        let error = talent_error(TalentError::NotAPosition("这是一份简历".to_owned()));
        assert_eq!(error.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error.code(), "not_a_position");
        assert!(error.message().contains("这是一份简历"));

        let error = talent_error(TalentError::Llm(LlmError::Auth("sk-secret".to_owned())));
        assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error.code(), "llm_unavailable");
        // 原始报错只进日志，不返回给前端
        assert!(!error.message().contains("sk-secret"));
    }
}
