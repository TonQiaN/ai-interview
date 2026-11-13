//! 把长文本切成引擎一次能合成的小段。
//!
//! 优先在句子结束的地方切，一句话太长再在逗号处切，实在没有标点才硬切。
//! 标点留在前一段的末尾，这样每一段读出来的语气是完整的。

const SENTENCE_ENDS: &[char] = &['。', '！', '？', '!', '?', '；', ';', '…', '\n'];
const CLAUSE_ENDS: &[char] = &['，', ',', '、', '：', ':', ' '];

/// 把 `text` 切成每段不超过 `max_chars` 个字符的若干段。
///
/// 段首段尾的空白会去掉，空段会丢掉；文本本来就够短时原样返回一段。
/// `max_chars` 是 0 时当成 1 处理。
pub fn split_text(text: &str, max_chars: usize) -> Vec<String> {
    let max_chars = max_chars.max(1);
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }
    if text.chars().count() <= max_chars {
        return vec![text.to_owned()];
    }

    // 先切到最小的单位（句子，太长的句子再切成分句和硬切的片段），再尽量多地拼回去
    let mut units = Vec::new();
    for sentence in split_after(text, SENTENCE_ENDS) {
        if sentence.chars().count() <= max_chars {
            units.push(sentence);
            continue;
        }
        for clause in split_after(sentence, CLAUSE_ENDS) {
            if clause.chars().count() <= max_chars {
                units.push(clause);
            } else {
                units.extend(hard_split(clause, max_chars));
            }
        }
    }

    let mut segments = Vec::new();
    let mut current = String::new();
    let mut current_len = 0;
    for unit in units {
        let unit_len = unit.chars().count();
        if current_len + unit_len > max_chars && !current.is_empty() {
            push_trimmed(&mut segments, &current);
            current.clear();
            current_len = 0;
        }
        current.push_str(unit);
        current_len += unit_len;
    }
    push_trimmed(&mut segments, &current);
    segments
}

fn push_trimmed(segments: &mut Vec<String>, text: &str) {
    let text = text.trim();
    if !text.is_empty() {
        segments.push(text.to_owned());
    }
}

/// 在每个分隔符的后面切开，分隔符留在前一片里。连续的分隔符（比如「？！」）不拆开。
fn split_after<'a>(text: &'a str, delimiters: &[char]) -> Vec<&'a str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if !delimiters.contains(&ch) {
            continue;
        }
        let next_is_delimiter = chars
            .peek()
            .is_some_and(|(_, next)| delimiters.contains(next));
        if next_is_delimiter {
            continue;
        }
        let end = index + ch.len_utf8();
        pieces.push(&text[start..end]);
        start = end;
    }
    if start < text.len() {
        pieces.push(&text[start..]);
    }
    pieces
}

fn hard_split(text: &str, max_chars: usize) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut count = 0;
    for (index, _) in text.char_indices() {
        if count == max_chars {
            pieces.push(&text[start..index]);
            start = index;
            count = 0;
        }
        count += 1;
    }
    pieces.push(&text[start..]);
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lens(segments: &[String]) -> Vec<usize> {
        segments.iter().map(|s| s.chars().count()).collect()
    }

    #[test]
    fn short_text_stays_in_one_piece() {
        assert_eq!(
            split_text("  请先做一个自我介绍。 ", 350),
            vec!["请先做一个自我介绍。"]
        );
        assert!(split_text("  \n ", 350).is_empty());
    }

    #[test]
    fn splits_at_sentence_ends_and_packs_as_many_sentences_as_fit() {
        let text = "第一句话。第二句话！第三句话？第四句话。";
        // 每句 5 个字，上限 12：两句一段
        assert_eq!(
            split_text(text, 12),
            vec!["第一句话。第二句话！", "第三句话？第四句话。"]
        );
        // 上限 5：一句一段
        assert_eq!(split_text(text, 5).len(), 4);
    }

    #[test]
    fn a_long_sentence_is_split_at_commas() {
        let text = "你在上一家公司负责过哪些项目，其中最有挑战的是哪一个，你是怎么解决的？";
        let segments = split_text(text, 16);
        assert_eq!(
            segments,
            vec![
                "你在上一家公司负责过哪些项目，",
                "其中最有挑战的是哪一个，",
                "你是怎么解决的？"
            ]
        );
    }

    #[test]
    fn text_without_punctuation_is_cut_hard_and_nothing_is_lost() {
        let text = "好".repeat(1000);
        let segments = split_text(&text, 350);
        assert_eq!(lens(&segments), vec![350, 350, 300]);
        assert_eq!(segments.concat(), text);
    }

    #[test]
    fn every_segment_respects_the_limit_and_the_text_is_preserved() {
        let text = "Tell me about a project you led. What went wrong, and how did you fix it? \
                    请用中文再回答一遍：你从中学到了什么？谢谢！"
            .repeat(7);
        for max in [1, 7, 20, 64, 350] {
            let segments = split_text(&text, max);
            assert!(lens(&segments).iter().all(|&len| len <= max), "上限 {max}");
            let joined: String = segments.concat().split_whitespace().collect();
            let original: String = text.split_whitespace().collect();
            assert_eq!(joined, original, "上限 {max}");
        }
    }

    #[test]
    fn consecutive_punctuation_stays_together() {
        assert_eq!(
            split_text("真的吗？！那太好了……我们继续。", 8),
            vec!["真的吗？！", "那太好了……", "我们继续。"]
        );
    }

    #[test]
    fn zero_limit_does_not_loop_forever() {
        assert_eq!(split_text("你好", 0), vec!["你", "好"]);
    }
}
