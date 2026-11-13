//! 去掉 serde 报错里带着的模型输出。
//!
//! serde 的类型错误会把出错的值原样带上，比如模型把工作经历写成了一段话，报错就是
//! `invalid type: string "在某公司负责……", expected a sequence`。模型输出里常常是候选人的
//! 简历、回答和评价，这些不能进日志，也不能跟着错误传到调用方的日志和任务失败原因里。
//! 所以写日志和返回错误之前先过一遍这里，只留「是什么类型、想要什么类型」。

/// 去掉 serde 报错里带的值。
///
/// - `invalid type: string "……", expected a sequence`、`invalid value: integer `5`, expected u8`：
///   中间那段是模型输出的原文，只留类型的名字。
/// - `unknown variant `……`, expected one of …`、`unknown field `……`, expected …`：
///   反引号里是模型写的词，去掉；后面列出来的是我们自己类型里的名字，留着。
/// - 其他错误（缺字段、语法不对）带的是字段名和位置，不是内容，但里面如果有双引号括起来的
///   文字也去掉。
pub(crate) fn without_values(error: &str) -> String {
    for prefix in ["invalid type: ", "invalid value: "] {
        let Some(rest) = error.strip_prefix(prefix) else {
            continue;
        };
        // 值里面可能也有 ", expected "，所以从最后一个算起
        let (unexpected, expected) = split_expected(rest);
        let kind: String = unexpected
            .chars()
            .take_while(|ch| !matches!(ch, '"' | '`') && !ch.is_ascii_digit() && *ch != '-')
            .collect();
        return join_expected(&format!("{prefix}{}", kind.trim()), expected);
    }
    for prefix in ["unknown variant ", "unknown field "] {
        let Some(rest) = error.strip_prefix(prefix) else {
            continue;
        };
        let (_, expected) = split_expected(rest);
        if expected.is_empty() {
            // 形如 unknown variant `x`, there are no variants：位置信息在最后，留着
            let position = rest.rfind(" at line ").map_or("", |at| &rest[at..]);
            return format!("{}{position}", prefix.trim_end());
        }
        return join_expected(prefix.trim_end(), expected);
    }
    strip_quoted(error)
}

fn split_expected(rest: &str) -> (&str, &str) {
    rest.rsplit_once(", expected ").unwrap_or((rest, ""))
}

fn join_expected(head: &str, expected: &str) -> String {
    if expected.is_empty() {
        head.to_owned()
    } else {
        format!("{head}, expected {expected}")
    }
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

    use super::*;

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum Level {
        Junior,
        Senior,
    }

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Sample {
        #[allow(dead_code, reason = "只用来触发类型错误")]
        items: Vec<String>,
        #[allow(dead_code, reason = "只用来触发类型错误")]
        #[serde(default)]
        years: Option<u8>,
        #[allow(dead_code, reason = "只用来触发类型错误")]
        #[serde(default)]
        level: Option<Level>,
    }

    /// 和 `chat_json` 一样从文字解析，报错里带着行列位置。
    fn error_for(json: &str) -> String {
        let error = serde_json::from_str::<Sample>(json).expect_err("类型不对");
        without_values(&error.to_string())
    }

    #[test]
    fn type_errors_keep_the_kinds_and_drop_the_value() {
        let message = error_for(r#"{"items": "负责订单结算, expected 很多 \"引号\""}"#);
        assert!(
            message.starts_with("invalid type: string, expected a sequence at line 1"),
            "{message}"
        );

        let message = error_for(r#"{"items": [], "years": 13800001234}"#);
        assert!(!message.contains("1380000"), "{message}");
        assert!(message.starts_with("invalid value: integer"), "{message}");

        let message = error_for(r#"{"items": [], "years": -13800001234}"#);
        assert!(!message.contains("1380000"), "{message}");

        let message = error_for(r#"{"items": [], "years": "八年"}"#);
        assert!(!message.contains("八年"), "{message}");
    }

    #[test]
    fn unknown_names_written_by_the_model_are_dropped() {
        let message = error_for(r#"{"items": [], "level": "在某公司做到了总监"}"#);
        assert!(!message.contains("总监"), "{message}");
        assert!(
            message.starts_with("unknown variant, expected `junior` or `senior`"),
            "{message}"
        );

        let message = error_for(r#"{"items": [], "张三的手机": 1}"#);
        assert!(!message.contains("张三"), "{message}");
        assert!(
            message.starts_with("unknown field, expected one of `items`"),
            "{message}"
        );
    }

    #[test]
    fn other_errors_keep_field_names_and_positions() {
        let message = error_for("{}");
        assert!(message.starts_with("missing field `items`"), "{message}");

        let message = error_for(r#"{"items": ["#);
        assert!(message.starts_with("EOF while parsing"), "{message}");

        assert_eq!(strip_quoted(r#"a "b \" c" d"#), "a … d");
    }
}
