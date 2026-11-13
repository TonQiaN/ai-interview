//! 引擎的错误。

use interview_llm::LlmError;

use crate::config::ConfigIssue;
use crate::plan::PlanIssue;

/// 引擎的错误。
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// 面试配置不合要求。
    #[error("面试配置有 {} 处问题，第一处：{}", .0.len(), first_config_issue(.0))]
    InvalidConfig(Vec<ConfigIssue>),

    /// 出题的输入不够，例如岗位描述是空的。
    #[error("出题的输入不完整：{0}")]
    InvalidInput(String),

    /// 面试计划本身有硬伤。
    #[error("面试计划有 {} 处问题，第一处：{}", .0.len(), first_plan_issue(.0))]
    InvalidPlan(Vec<PlanIssue>),

    /// 会话状态和这次调用对不上，例如面试已经结束了还在推进。
    #[error("会话状态不对：{0}")]
    InvalidState(String),

    /// 调用大模型失败。
    #[error(transparent)]
    Llm(#[from] LlmError),
}

fn first_config_issue(issues: &[ConfigIssue]) -> String {
    issues
        .first()
        .map(|issue| format!("{}：{}", issue.field, issue.message))
        .unwrap_or_default()
}

fn first_plan_issue(issues: &[PlanIssue]) -> String {
    issues
        .first()
        .map(|issue| format!("{}：{}", issue.location, issue.message))
        .unwrap_or_default()
}
