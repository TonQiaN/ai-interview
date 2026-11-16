//! 报告正文的收尾：把模型偶尔写进正文的题目 id 换成「第几题」。
//!
//! 提示词已经要求正文里不写 id，实测绝大多数时候模型照做，但偶尔还是会写出「q3 作答期间」这样的话。
//! HR 看不懂 id，而 id 对应第几题是确定的事，所以由程序兜底换掉，不再为这个去调一次模型。
//!
//! 事件种类和字段的英文名（`page_taken_over`、`video_reduced` 这类）没有这样确定的换法：
//! 「interview_started 的 video_reduced 为 true」要整句改写才通顺。这一类靠提示词管，
//! [`program_names_in_prose`] 只负责数出来，评估时看提示词管没管住。

use std::collections::BTreeSet;

use serde_json::Value;

use crate::record::InterviewRecord;
use crate::report::InterviewReport;
use crate::transcript::QuestionGroup;

/// 这些字段里放的是 id、原话或者给程序看的数据，不动。
const KEPT_FIELDS: [&str; 7] = [
    "interview_id",
    "question_id",
    "segment_id",
    "segment_ids",
    "event_id",
    "quote",
    "meta",
];

/// 把正文里的题目 id 换成「第 N 题」，追问的 id 换成「第 N 题的追问」。
///
/// 只换带数字的 id（面试引擎起的是 `q3`、`q3.f1` 这样的），并且要求 id 前后不是字母、数字，
/// 免得把一个普通的词换掉。
pub(crate) fn replace_question_ids(
    report: InterviewReport,
    groups: &[QuestionGroup<'_>],
) -> InterviewReport {
    let mut names: Vec<(&str, String)> = Vec::new();
    for (index, group) in groups.iter().enumerate() {
        let number = index + 1;
        names.push((group.root.id.as_str(), format!("第 {number} 题")));
        for follow_up in &group.follow_ups {
            names.push((follow_up.id.as_str(), format!("第 {number} 题的追问")));
        }
    }
    names.retain(|(id, _)| id.chars().any(|c| c.is_ascii_digit()));
    // 长的先换，`q1.f1` 才不会被当成 `q1` 换掉一半
    names.sort_by_key(|(id, _)| std::cmp::Reverse(id.len()));
    if names.is_empty() {
        return report;
    }

    let Ok(mut value) = serde_json::to_value(&report) else {
        return report;
    };
    rewrite(&mut value, &names);
    serde_json::from_value(value).unwrap_or(report)
}

fn rewrite(value: &mut Value, names: &[(&str, String)]) {
    match value {
        Value::String(text) => {
            for (id, name) in names {
                if text.contains(id) {
                    *text = replace_token(text, id, name);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| rewrite(item, names)),
        Value::Object(fields) => {
            for (key, field) in fields.iter_mut() {
                if !KEPT_FIELDS.contains(&key.as_str()) {
                    rewrite(field, names);
                }
            }
        }
        _ => {}
    }
}

/// 把 `text` 里单独出现的 `id` 换成 `name`。前后紧挨着字母、数字、下划线、连字符的不算单独出现。
/// 换上去的是中文，原来 id 两边的空格一起去掉。
fn replace_token(text: &str, id: &str, name: &str) -> String {
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    // 紧挨在 `rest` 前面的那个字符是不是字母数字。只有上一处没换、原样留下 id 的时候才是
    let mut after_word = false;
    while let Some(position) = rest.find(id) {
        let (head, tail) = rest.split_at(position);
        let after = &tail[id.len()..];
        let before_is_word = head.chars().next_back().map_or(after_word, is_word);
        let next_is_word = after.chars().next().is_some_and(is_word);
        out.push_str(head);
        if before_is_word || next_is_word {
            out.push_str(id);
            rest = after;
            after_word = true;
        } else {
            if out.ends_with(' ') {
                out.pop();
            }
            out.push_str(name);
            rest = after.strip_prefix(' ').unwrap_or(after);
            after_word = false;
        }
    }
    out.push_str(rest);
    out
}

/// 数出报告正文里给程序用的英文名：过程事件的种类和字段（`page_taken_over`、`video_reduced`）、
/// 报告自己的字段名（`risk_level`）。这些是提示词里讲给模型听的，HR 看不懂，不该出现在正文里。
///
/// 返回每一处出现的名字，按出现的先后。评估用，分析本身不调用。
///
/// 怎么认：正文里一串连着的字母、数字、下划线，全是小写并且带下划线的，或者正好是这场面试某个
/// 过程事件的种类或字段名的，算一处。候选人自己说过的词不算（比如 SQL 里的 `row_number`）：
/// 在岗位、简历、题目、转写里出现过的都放过。整个字段只有这一个词的是枚举值，也不算。
pub fn program_names_in_prose(record: &InterviewRecord, report: &InterviewReport) -> Vec<String> {
    let mut event_names = BTreeSet::new();
    for event in &record.events {
        event_names.insert(event.kind.clone());
        let detail = event.detail.as_deref().map(serde_json::from_str::<Value>);
        if let Some(Ok(Value::Object(fields))) = detail {
            event_names.extend(fields.keys().cloned());
        }
    }
    // 候选人和 HR 写的、说的内容。事件不算在内
    let mut material = serde_json::to_value(record).unwrap_or_default();
    if let Value::Object(fields) = &mut material {
        fields.remove("events");
    }
    let material = material.to_string();

    let mut found = Vec::new();
    if let Ok(value) = serde_json::to_value(report) {
        collect_program_names(&value, &event_names, &material, &mut found);
    }
    found
}

fn collect_program_names(
    value: &Value,
    event_names: &BTreeSet<String>,
    material: &str,
    found: &mut Vec<String>,
) {
    match value {
        Value::String(text) => {
            let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
            for word in text.split(|c: char| !is_word(c)) {
                // 整个字段就是这一个词：枚举值
                if word.is_empty() || word.len() == text.len() {
                    continue;
                }
                let snake_case = word.contains('_')
                    && word.starts_with(|c: char| c.is_ascii_lowercase())
                    && !word.ends_with('_')
                    && !word.chars().any(|c| c.is_ascii_uppercase());
                if (snake_case || event_names.contains(word)) && !material.contains(word) {
                    found.push(word.to_owned());
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_program_names(item, event_names, material, found);
            }
        }
        Value::Object(fields) => {
            for (key, field) in fields {
                if !KEPT_FIELDS.contains(&key.as_str()) {
                    collect_program_names(field, event_names, material, found);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found_in(text: &str, event_names: &[&str], material: &str) -> Vec<String> {
        let event_names = event_names.iter().map(|name| (*name).to_owned()).collect();
        let value = serde_json::json!({
            "integrity": {"summary": text, "signals": [{"kind": "page_event", "quote": "page_hidden"}]},
            "meta": {"note": "risk_level"},
        });
        let mut found = Vec::new();
        collect_program_names(&value, &event_names, material, &mut found);
        found
    }

    #[test]
    fn event_and_field_names_in_prose_are_counted() {
        assert_eq!(
            found_in(
                "02:05 换到了另一个页面（page_taken_over），interview_started 的 resumed 为 true，risk_level 不变。",
                &["page_taken_over", "interview_started", "resumed"],
                "",
            ),
            [
                "page_taken_over",
                "interview_started",
                "resumed",
                "risk_level"
            ]
        );
    }

    #[test]
    fn words_from_the_interview_itself_and_enum_values_are_not_counted() {
        // 候选人说过的词、普通的英文词、带大写的名字、枚举值和引文都不算
        assert_eq!(
            found_in(
                "候选人讲了用 row_number 去重，用的是 Redis 和 MAX_CONN，换页面前后水平一致。",
                &["page_taken_over"],
                r#"{"transcript":[{"text":"我用 row_number 开窗去重"}]}"#,
            ),
            Vec::<String>::new()
        );
    }

    #[test]
    fn standalone_ids_are_replaced() {
        assert_eq!(
            replace_token("录像抽帧显示 q3 作答期间画面里有两个人", "q3", "第 3 题"),
            "录像抽帧显示第 3 题作答期间画面里有两个人"
        );
        assert_eq!(
            replace_token("（q2「你就说用消息队列削峰」、q3）", "q2", "第 2 题"),
            "（第 2 题「你就说用消息队列削峰」、q3）"
        );
        assert_eq!(replace_token("q1", "q1", "第 1 题"), "第 1 题");
        assert_eq!(
            replace_token("q1、q1 都一样", "q1", "第 1 题"),
            "第 1 题、第 1 题都一样"
        );
    }

    #[test]
    fn ids_inside_other_words_are_kept() {
        assert_eq!(replace_token("q10 没问到", "q1", "第 1 题"), "q10 没问到");
        assert_eq!(
            replace_token("用的是 mq1 集群", "q1", "第 1 题"),
            "用的是 mq1 集群"
        );
        assert_eq!(replace_token("q1q1", "q1", "第 1 题"), "q1q1");
    }
}
