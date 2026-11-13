//! 宽容地读大模型输出的 JSON。
//!
//! 大模型偶尔会把「没有」写成 `null`，或者把一个数字写成字符串。这些不值得让整次解析失败、
//! 再花一次调用去纠正，所以在反序列化这一层消化掉。

use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// `null` 当成类型的默认值。配合 `#[serde(default)]` 用，字段缺失和 `null` 效果一样。
pub(crate) fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// 可有可无的文字：空白当成没有，数字和布尔值转成文字。
pub(crate) fn text<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match Value::deserialize(deserializer)? {
        Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    })
}

/// 可有可无的数字：能读成数字的字符串也算。
pub(crate) fn number<'de, D>(deserializer: D) -> Result<Option<f32>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match Value::deserialize(deserializer)? {
        // 工作年限这类数字很小，只要一位小数的精度，缩成 f32 没有问题
        Value::Number(number) => number.as_f64().map(|value| value as f32),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    })
}

/// 一组文字：`null` 当成空，单个字符串当成只有一项，空白项去掉。
pub(crate) fn text_list<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let items = match Value::deserialize(deserializer)? {
        Value::Array(items) => items,
        Value::Null => Vec::new(),
        single => vec![single],
    };
    Ok(items
        .into_iter()
        .filter_map(|item| match item {
            Value::String(text) => Some(text),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        })
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .collect())
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    #[derive(Debug, Default, Deserialize)]
    struct Sample {
        #[serde(default, deserialize_with = "super::text")]
        title: Option<String>,
        #[serde(default, deserialize_with = "super::number")]
        years: Option<f32>,
        #[serde(default, deserialize_with = "super::text_list")]
        skills: Vec<String>,
        #[serde(default, deserialize_with = "super::null_as_default")]
        summary: String,
    }

    #[test]
    fn null_and_missing_fields_become_empty() {
        let sample: Sample =
            serde_json::from_str(r#"{"title": null, "skills": null, "summary": null}"#)
                .expect("能解析");
        assert_eq!(sample.title, None);
        assert_eq!(sample.years, None);
        assert!(sample.skills.is_empty());
        assert_eq!(sample.summary, "");
    }

    #[test]
    fn loose_values_are_accepted() {
        let sample: Sample = serde_json::from_str(
            r#"{"title": "  后端工程师 ", "years": "7.5", "skills": ["Rust", " ", 3, null], "summary": "稳"}"#,
        )
        .expect("能解析");
        assert_eq!(sample.title.as_deref(), Some("后端工程师"));
        assert_eq!(sample.years, Some(7.5));
        assert_eq!(sample.skills, ["Rust", "3"]);
        assert_eq!(sample.summary, "稳");
    }

    #[test]
    fn a_single_string_counts_as_a_one_item_list() {
        let sample: Sample = serde_json::from_str(r#"{"skills": "Rust"}"#).expect("能解析");
        assert_eq!(sample.skills, ["Rust"]);
    }
}
