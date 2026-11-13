//! 报告正文的收尾：把模型偶尔写进正文的题目 id 换成「第几题」。
//!
//! 提示词已经要求正文里不写 id，实测绝大多数时候模型照做，但偶尔还是会写出「q3 作答期间」这样的话。
//! HR 看不懂 id，而 id 对应第几题是确定的事，所以由程序兜底换掉，不再为这个去调一次模型。

use serde_json::Value;

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

#[cfg(test)]
mod tests {
    use super::*;

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
