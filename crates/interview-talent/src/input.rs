//! 收拾接口收到的文字：去空白、限长度、区分「没传」和「清空」。

use interview_platform::{ApiError, ApiResult};
use serde::{Deserialize, Deserializer};

/// PATCH 里可以清空的字段：没出现是 `None`，写了 `null` 是 `Some(None)`，有值是 `Some(Some(值))`。
pub(crate) fn nullable<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// 短文字（名字、部门、城市这类）最多这么多字。
pub(crate) const SHORT_TEXT_MAX: usize = 200;
/// 长文字（岗位描述、备注）最多这么多字。
pub(crate) const LONG_TEXT_MAX: usize = 50_000;

/// 可有可无的文字：去掉首尾空白，空的当作没有，超长报 422。
pub(crate) fn optional(
    field: &str,
    value: Option<String>,
    max_chars: usize,
) -> ApiResult<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    check_length(field, trimmed, max_chars)?;
    Ok(Some(trimmed.to_owned()))
}

/// 必填的文字：去掉首尾空白，空的报 422。
pub(crate) fn required(field: &str, value: &str, max_chars: usize) -> ApiResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::invalid_field(field, "不能为空"));
    }
    check_length(field, trimmed, max_chars)?;
    Ok(trimmed.to_owned())
}

/// 可以留空的长文字：去掉首尾空白，超长报 422。
pub(crate) fn body(field: &str, value: &str, max_chars: usize) -> ApiResult<String> {
    let trimmed = value.trim();
    check_length(field, trimmed, max_chars)?;
    Ok(trimmed.to_owned())
}

fn check_length(field: &str, value: &str, max_chars: usize) -> ApiResult<()> {
    if value.chars().count() > max_chars {
        return Err(ApiError::invalid_field(
            field,
            format!("不能超过 {max_chars} 字"),
        ));
    }
    Ok(())
}

/// 把关键词变成 `ILIKE` 的模式：前后加 `%`，关键词里的 `%`、`_`、`\` 当普通字符。
pub(crate) fn like_pattern(keyword: &str) -> String {
    let mut pattern = String::with_capacity(keyword.len() + 2);
    pattern.push('%');
    for c in keyword.chars() {
        if matches!(c, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push('%');
    pattern
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default, Deserialize)]
    struct Patch {
        #[serde(default, deserialize_with = "nullable")]
        department: Option<Option<String>>,
    }

    #[test]
    fn patch_tells_missing_from_null() {
        let missing: Patch = serde_json::from_str("{}").expect("能解析");
        assert_eq!(missing.department, None);
        let cleared: Patch = serde_json::from_str(r#"{"department": null}"#).expect("能解析");
        assert_eq!(cleared.department, Some(None));
        let set: Patch = serde_json::from_str(r#"{"department": "平台部"}"#).expect("能解析");
        assert_eq!(set.department, Some(Some("平台部".to_owned())));
    }

    #[test]
    fn text_is_trimmed_and_limited() {
        assert_eq!(
            optional("a", Some("  ".to_owned()), 10).expect("合法"),
            None
        );
        assert_eq!(
            optional("a", Some(" 上海 ".to_owned()), 10).expect("合法"),
            Some("上海".to_owned())
        );
        assert!(optional("a", Some("字".repeat(11)), 10).is_err());
        assert!(required("title", "   ", 10).is_err());
        assert_eq!(body("description", " \n", 10).expect("合法"), "");
    }

    #[test]
    fn like_pattern_escapes_wildcards() {
        assert_eq!(like_pattern("50%_a\\"), "%50\\%\\_a\\\\%");
    }
}
