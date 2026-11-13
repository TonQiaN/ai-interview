//! 宽松的反序列化。
//!
//! RAAS 的字段类型不稳定：同一个字段有时是数字有时是字符串，列表有时是 `null`，
//! 历史数据里还有空字符串。这里统一收敛成 `Option<String>`、`Option<f64>`、`Vec<String>`，
//! 让上层不用关心这些差异，也不会因为一个字段的类型变了就整条记录解析失败。

use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// 字符串、数字、布尔都收成文本；`null`、空白字符串、对象和数组收成 `None`。
pub(crate) fn text<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Ok(value_to_text(&Value::deserialize(deserializer)?))
}

/// 数字或写成字符串的数字；其他都是 `None`。
pub(crate) fn number<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<f64>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    })
}

/// 文本列表。接受数组（元素按 [`text`] 的规则转换，转不了的丢掉）、单个文本、`null`。
pub(crate) fn text_list<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<String>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::Array(items) => items.iter().filter_map(value_to_text).collect(),
        other => value_to_text(&other).into_iter().collect(),
    })
}

/// 对象列表。`null` 当空列表；数组里解析不了的元素丢掉，不让一条坏数据拖垮整条记录。
pub(crate) fn list<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    Ok(match Value::deserialize(deserializer)? {
        Value::Array(items) => items
            .into_iter()
            .filter_map(|item| serde_json::from_value(item).ok())
            .collect(),
        _ => Vec::new(),
    })
}

/// 一个 JSON 对象的全部字段，用来处理「同一个意思有几个键名」的条目。
#[derive(Deserialize)]
#[serde(transparent)]
pub(crate) struct Fields(serde_json::Map<String, Value>);

impl Fields {
    /// 按顺序取第一个有值的键。
    pub(crate) fn text(&self, keys: &[&str]) -> Option<String> {
        keys.iter()
            .find_map(|key| self.0.get(*key).and_then(value_to_text))
    }

    /// 按顺序取第一个非空的列表。
    pub(crate) fn text_list(&self, keys: &[&str]) -> Vec<String> {
        keys.iter()
            .filter_map(|key| self.0.get(*key))
            .map(|value| match value {
                Value::Array(items) => items.iter().filter_map(value_to_text).collect(),
                other => value_to_text(other).into_iter().collect::<Vec<_>>(),
            })
            .find(|list| !list.is_empty())
            .unwrap_or_default()
    }
}

fn value_to_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::json;

    #[derive(Debug, Deserialize)]
    struct Probe {
        #[serde(default, deserialize_with = "super::text")]
        text: Option<String>,
        #[serde(default, deserialize_with = "super::number")]
        number: Option<f64>,
        #[serde(default, deserialize_with = "super::text_list")]
        tags: Vec<String>,
        #[serde(default, deserialize_with = "super::list")]
        items: Vec<Item>,
    }

    #[derive(Debug, Deserialize)]
    struct Item {
        id: u32,
    }

    fn probe(value: serde_json::Value) -> Probe {
        serde_json::from_value(value).expect("宽松解析不应失败")
    }

    #[test]
    fn missing_and_null_fields_become_empty() {
        for value in [
            json!({}),
            json!({"text": null, "number": null, "tags": null, "items": null}),
        ] {
            let p = probe(value);
            assert_eq!(p.text, None);
            assert_eq!(p.number, None);
            assert!(p.tags.is_empty());
            assert!(p.items.is_empty());
        }
    }

    #[test]
    fn scalars_are_coerced() {
        let p = probe(json!({"text": 3900, "number": "6000.5", "tags": "Rust"}));
        assert_eq!(p.text.as_deref(), Some("3900"));
        assert_eq!(p.number, Some(6000.5));
        assert_eq!(p.tags, ["Rust"]);
    }

    #[test]
    fn blanks_and_bad_elements_are_dropped() {
        let p = probe(json!({
            "text": "   ",
            "number": "面议",
            "tags": ["Go", "", null, 7, {"x": 1}],
            "items": [{"id": 1}, {"id": "不是数字"}, "乱入", {"id": 2}],
        }));
        assert_eq!(p.text, None);
        assert_eq!(p.number, None);
        assert_eq!(p.tags, ["Go", "7"]);
        assert_eq!(p.items.iter().map(|i| i.id).collect::<Vec<_>>(), [1, 2]);
    }
}
