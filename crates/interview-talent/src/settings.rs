//! 这个 crate 调大模型时的设置。

use interview_llm::{Effort, Thinking};

use crate::error::TalentError;

/// 读简历、整理岗位描述时的思考设置的环境变量名。
pub const READING_THINKING_ENV_VAR: &str = "AI_INTERVIEW_TALENT_THINKING";
/// 生成候选人画像时的思考设置的环境变量名。
pub const PROFILE_THINKING_ENV_VAR: &str = "AI_INTERVIEW_TALENT_PROFILE_THINKING";

/// 调大模型时的设置。思考开关和强度可以分开配：照着原文整理不需要多想，写画像值得想一想。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TalentAiSettings {
    /// 读简历、整理岗位描述。默认关闭思考：这是照着原文整理，关掉更快也更省。
    pub reading_thinking: Thinking,
    /// 生成候选人画像、岗位匹配分析。默认开启、强度 low。
    pub profile_thinking: Thinking,
}

impl Default for TalentAiSettings {
    fn default() -> Self {
        Self {
            reading_thinking: Thinking::off(),
            profile_thinking: Thinking::on(Effort::Low),
        }
    }
}

impl TalentAiSettings {
    /// 从环境变量读。取值 `off`、`low`、`high`、`max`，不填用默认值。
    pub fn from_env() -> Result<Self, TalentError> {
        let defaults = Self::default();
        Ok(Self {
            reading_thinking: thinking_from_env(READING_THINKING_ENV_VAR)?
                .unwrap_or(defaults.reading_thinking),
            profile_thinking: thinking_from_env(PROFILE_THINKING_ENV_VAR)?
                .unwrap_or(defaults.profile_thinking),
        })
    }
}

fn thinking_from_env(name: &str) -> Result<Option<Thinking>, TalentError> {
    match std::env::var(name) {
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => parse_thinking(&value)
            .map(Some)
            .ok_or_else(|| TalentError::Config(format!("{name} 只能是 off、low、high 或 max"))),
        Err(_) => Ok(None),
    }
}

/// 把 `off`、`low`、`high`、`max` 读成思考设置。
pub fn parse_thinking(value: &str) -> Option<Thinking> {
    match value.trim().to_ascii_lowercase().as_str() {
        "off" => Some(Thinking::off()),
        "low" => Some(Thinking::on(Effort::Low)),
        "high" => Some(Thinking::on(Effort::High)),
        "max" => Some(Thinking::on(Effort::Max)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_four_levels() {
        assert_eq!(parse_thinking("off"), Some(Thinking::off()));
        assert_eq!(parse_thinking(" Low "), Some(Thinking::on(Effort::Low)));
        assert_eq!(parse_thinking("HIGH"), Some(Thinking::on(Effort::High)));
        assert_eq!(parse_thinking("max"), Some(Thinking::on(Effort::Max)));
        assert_eq!(parse_thinking("medium"), None);
    }
}
