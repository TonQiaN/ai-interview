//! 让大模型输出 JSON，读不成想要的类型时，报错里不带输出的原文。
//!
//! `interview_llm::chat_json` 读不成目标类型时，会把 serde 的报错写进日志、发回给模型、
//! 最后放进返回的错误里。serde 的类型错误会把出错的值原样带上，比如模型把工作经历写成了
//! 一段话，报错就是 `invalid type: string "在某公司负责……", expected a sequence`。
//! 我们这里模型输出的是简历和画像，这些不能进日志，所以先读成不挑类型的 JSON，
//! 类型检查自己做，报错只留「什么类型、想要什么类型」。

use interview_llm::{
    ChatModel, ChatRequest, DEFAULT_JSON_ATTEMPTS, LlmError, Usage, chat_json_checked,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

/// 要求模型输出 JSON 并读成 `T`，再用 `check` 检查内容。
///
/// 读不成 `T` 或 `check` 不通过时，把原因告诉模型让它改，最多 [`DEFAULT_JSON_ATTEMPTS`] 次。
/// `check` 返回的话会原样发给模型、写进日志，所以不要在里面引用模型输出的内容。
pub(crate) async fn chat_json_private<T, F>(
    model: &dyn ChatModel,
    request: ChatRequest,
    check: F,
) -> Result<(T, Usage), LlmError>
where
    T: DeserializeOwned,
    F: Fn(&T) -> Result<(), String> + Send + Sync,
{
    let reply = chat_json_checked::<Value, _>(model, request, DEFAULT_JSON_ATTEMPTS, |value| {
        let typed = T::deserialize(value).map_err(|error| describe(&error.to_string()))?;
        check(&typed)
    })
    .await?;
    // 上面已经读成功过一次，这里不会失败；万一失败也只报类型，不带内容
    let typed = T::deserialize(&reply.value).map_err(|error| LlmError::InvalidJson {
        attempts: reply.attempts,
        message: describe(&error.to_string()),
    })?;
    Ok((typed, reply.usage))
}

/// 把 serde 的报错变成不带值的说法。
fn describe(error: &str) -> String {
    format!("JSON 的结构不对：{}", without_values(error))
}

/// 去掉 serde 报错里带的值。
///
/// 类型和取值的错误形如 `invalid type: string "……", expected a sequence`、
/// `invalid value: integer `5`, expected ……`：中间那段是模型输出的原文，只留类型的名字。
/// 其他错误（缺字段、不认识的字段）带的是字段名，不是内容，但里面如果有引号括起来的文字也去掉。
fn without_values(error: &str) -> String {
    for prefix in ["invalid type: ", "invalid value: ", "invalid length "] {
        let Some(rest) = error.strip_prefix(prefix) else {
            continue;
        };
        // 值里面可能也有 ", expected "，所以从最后一个算起
        let (unexpected, expected) = rest.rsplit_once(", expected ").unwrap_or((rest, ""));
        let kind: String = unexpected
            .chars()
            .take_while(|ch| !matches!(ch, '"' | '`') && !ch.is_ascii_digit())
            .collect();
        let kind = kind.trim();
        return if expected.is_empty() {
            format!("{prefix}{kind}")
        } else {
            format!("{prefix}{kind}, expected {expected}")
        };
    }
    strip_quoted(error)
}

/// 去掉双引号括起来的部分（serde 用调试格式输出字符串，里面的引号是转义过的）。
fn strip_quoted(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '"' {
            kept.push(ch);
            continue;
        }
        kept.push('…');
        // 跳到配对的引号
        while let Some(inner) = chars.next() {
            match inner {
                '\\' => {
                    chars.next();
                }
                '"' => break,
                _ => {}
            }
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::json;

    use super::*;

    #[derive(Debug, Deserialize)]
    struct Sample {
        #[allow(dead_code, reason = "只用来触发类型错误")]
        items: Vec<String>,
        #[allow(dead_code, reason = "只用来触发类型错误")]
        #[serde(default)]
        years: Option<u8>,
    }

    fn error_for(value: Value) -> String {
        let error = Sample::deserialize(&value).expect_err("类型不对");
        without_values(&error.to_string())
    }

    #[test]
    fn type_errors_keep_the_kinds_and_drop_the_value() {
        let message = error_for(json!({"items": "负责订单结算, expected 很多 \"引号\""}));
        assert_eq!(message, "invalid type: string, expected a sequence");

        let message = error_for(json!({"items": [], "years": 13_800_001_234_u64}));
        assert!(!message.contains("1380000"), "{message}");
        assert!(message.starts_with("invalid value: integer"), "{message}");

        let message = error_for(json!({"items": [], "years": "八年"}));
        assert!(!message.contains("八年"), "{message}");
    }

    #[test]
    fn other_errors_keep_field_names() {
        let message = error_for(json!({}));
        assert_eq!(message, "missing field `items`");
        assert_eq!(strip_quoted(r#"a "b \" c" d"#), "a … d");
    }
}
