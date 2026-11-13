//! 把识别模型输出的 token 拼成词。
//!
//! 识别模型给的是一串 token 和每个 token 开始的时间：中文一个字一个 token，
//! 英文一个单词拆成几个 token（`" t"`、`"e"`、`"ch"`…，开头带空格的是单词的第一个 token），
//! 标点也是 token。这里把它们拼成 [`Word`]，各个模型后端共用。

use crate::transcript::{Word, needs_space};

/// 一个字最长算多久。说话人在这个字后面停顿时，模型只给了下一个字的开始时间，
/// 不能把整段停顿都算成这个字的时长。
const FIRST_CHAR_MAX_MS: u64 = 400;
/// 词里每多一个字符，最长时长多加这么多（英文单词比单个汉字长）。
const EXTRA_CHAR_MAX_MS: u64 = 120;
/// 一个词无论多长，最长算这么久。
const WORD_MAX_MS: u64 = 2_000;

/// 识别模型输出的一个 token。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token<'a> {
    /// token 的原文。开头的空格或 `▁` 表示它是一个新词的开头。
    pub text: &'a str,
    /// 这个 token 开始的时间，毫秒。
    pub start_ms: u64,
}

/// 把 token 拼成词。`end_ms` 是这段话结束的时间，用来给最后一个词收尾。
///
/// - 中日韩文字一个字一个词；连续的拉丁字母 token 拼成一个单词。
/// - 标点跟在前一个词后面，不单独成词；开头的标点直接丢掉。
/// - 模型只给了开始时间：一个词的结束时间取下一个 token 的开始时间，但不超过按字数算的上限。
pub fn words_from_tokens(tokens: &[Token<'_>], end_ms: u64) -> Vec<Word> {
    let mut words: Vec<Word> = Vec::new();
    // 和 words 一一对应：这个词的结束时间定下来没有
    let mut closed: Vec<bool> = Vec::new();
    let mut break_before_next = true;
    let mut last_start = 0;

    for token in tokens {
        // 时间戳偶尔会倒退一点，按不早于前一个 token 处理
        let start_ms = token.start_ms.max(last_start);
        last_start = start_ms;

        let starts_word = token.text.starts_with([' ', '▁']);
        let piece = token.text.trim_matches([' ', '▁']);
        if piece.is_empty() {
            break_before_next = break_before_next || starts_word;
            continue;
        }

        if piece.chars().all(is_punctuation) {
            if let (Some(word), Some(closed)) = (words.last_mut(), closed.last_mut()) {
                word.text.push_str(piece);
                if !*closed {
                    word.end_ms = start_ms;
                    *closed = true;
                }
            }
            break_before_next = true;
            continue;
        }

        let continues_previous = !starts_word
            && !break_before_next
            && !piece.chars().next().is_some_and(is_cjk)
            && words
                .last()
                .and_then(|word| word.text.chars().next_back())
                .is_some_and(|c| !is_cjk(c));
        if continues_previous {
            if let Some(word) = words.last_mut() {
                word.text.push_str(piece);
            }
        } else {
            if let (Some(word), Some(closed)) = (words.last_mut(), closed.last_mut())
                && !*closed
            {
                word.end_ms = start_ms;
                *closed = true;
            }
            words.push(Word {
                text: piece.to_owned(),
                start_ms,
                end_ms: start_ms,
            });
            closed.push(false);
        }
        break_before_next = false;
    }

    if let (Some(word), Some(false)) = (words.last_mut(), closed.last()) {
        word.end_ms = end_ms;
    }
    for word in &mut words {
        let spoken_chars = word.text.chars().filter(|c| !is_punctuation(*c)).count() as u64;
        let longest = (FIRST_CHAR_MAX_MS + EXTRA_CHAR_MAX_MS * spoken_chars.saturating_sub(1))
            .min(WORD_MAX_MS);
        word.end_ms = word.end_ms.clamp(word.start_ms, word.start_ms + longest);
    }
    words
}

/// 把词接成一句话：英文单词之间加空格，中文之间不加。
pub fn join_words(words: &[Word]) -> String {
    let mut text = String::new();
    for word in words {
        if needs_space(text.chars().next_back(), word.text.chars().next()) {
            text.push(' ');
        }
        text.push_str(&word.text);
    }
    text
}

/// 把一段普通文字切成 [`words_from_tokens`] 能吃的 token（不带时间）。假后端和测试用。
pub(crate) fn plain_tokens(text: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut in_latin_word = false;
    for c in text.chars() {
        if c.is_whitespace() {
            in_latin_word = false;
        } else if is_punctuation(c) {
            tokens.push(c.to_string());
            in_latin_word = false;
        } else if is_cjk(c) {
            tokens.push(format!(" {c}"));
            in_latin_word = false;
        } else if in_latin_word {
            if let Some(last) = tokens.last_mut() {
                last.push(c);
            }
        } else {
            tokens.push(format!(" {c}"));
            in_latin_word = true;
        }
    }
    tokens
}

fn is_punctuation(c: char) -> bool {
    !c.is_alphanumeric() && c != '\''
}

/// 中日韩文字：一个字就是一个词。
fn is_cjk(c: char) -> bool {
    matches!(
        u32::from(c),
        0x3040..=0x30FF       // 日文假名
            | 0x3400..=0x4DBF // 扩展 A
            | 0x4E00..=0x9FFF // 常用汉字
            | 0xAC00..=0xD7AF // 韩文
            | 0xF900..=0xFAFF // 兼容汉字
            | 0x20000..=0x2A6DF // 扩展 B
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens<'a>(pairs: &'a [(&'a str, u64)]) -> Vec<Token<'a>> {
        pairs
            .iter()
            .map(|&(text, start_ms)| Token { text, start_ms })
            .collect()
    }

    fn texts(words: &[Word]) -> Vec<&str> {
        words.iter().map(|word| word.text.as_str()).collect()
    }

    #[test]
    fn chinese_is_one_word_per_character_and_punctuation_joins_the_previous_word() {
        // 真实模型输出的形状：每个字前面带空格，标点是单独的 token
        let words = words_from_tokens(
            &tokens(&[
                (" 调", 2120),
                (" 查", 2320),
                (" ，", 2520),
                (" 然", 2640),
                (" 后", 2720),
            ]),
            3000,
        );
        assert_eq!(texts(&words), ["调", "查，", "然", "后"]);
        assert_eq!((words[0].start_ms, words[0].end_ms), (2120, 2320));
        // 「查」到标点为止，不延伸到「然」
        assert_eq!((words[1].start_ms, words[1].end_ms), (2320, 2520));
        assert_eq!((words[2].start_ms, words[2].end_ms), (2640, 2720));
        assert_eq!((words[3].start_ms, words[3].end_ms), (2720, 3000));
        assert_eq!(join_words(&words), "调查，然后");
    }

    #[test]
    fn english_pieces_are_merged_into_whole_words() {
        let words = words_from_tokens(
            &tokens(&[
                (" 关", 4400),
                (" 于", 4520),
                (" t", 4680),
                ("e", 4800),
                ("ch", 4880),
                ("no", 5000),
                ("lo", 5120),
                ("g", 5160),
                ("y", 5200),
                (" p", 5360),
                ("ri", 5440),
                ("va", 5520),
                ("c", 5680),
                ("y", 5720),
                (" 的", 5800),
                (" 东", 5960),
                (" 西", 6080),
                (" 。", 6240),
            ]),
            6320,
        );
        assert_eq!(
            texts(&words),
            ["关", "于", "technology", "privacy", "的", "东", "西。"]
        );
        assert_eq!((words[2].start_ms, words[2].end_ms), (4680, 5360));
        assert_eq!((words[3].start_ms, words[3].end_ms), (5360, 5800));
        assert_eq!(join_words(&words), "关于technology privacy的东西。");
    }

    #[test]
    fn sentencepiece_markers_and_english_punctuation_work_too() {
        let words = words_from_tokens(
            &tokens(&[
                ("▁This", 800),
                ("▁is", 1320),
                ("▁a", 1720),
                ("▁t", 2080),
                ("ri", 2120),
                ("ck", 2280),
                ("y", 2400),
                ("▁question", 2680),
                ("▁.", 2920),
                ("▁", 2950),
                ("ok", 3000),
            ]),
            3040,
        );
        assert_eq!(
            texts(&words),
            ["This", "is", "a", "tricky", "question.", "ok"]
        );
        assert_eq!(join_words(&words), "This is a tricky question. ok");
    }

    #[test]
    fn a_pause_after_a_word_is_not_counted_as_part_of_the_word() {
        // 「了」之后停了 3 秒才说「。」，「了」不能算 3 秒长
        let words = words_from_tokens(
            &tokens(&[
                (" 好", 1000),
                (" 了", 1200),
                (" 。", 4200),
                (" really", 5000),
            ]),
            9000,
        );
        assert_eq!((words[1].start_ms, words[1].end_ms), (1200, 1600));
        // 英文单词按字母数放宽上限：400 + 120 × 5 = 1000
        assert_eq!((words[2].start_ms, words[2].end_ms), (5000, 6000));
    }

    #[test]
    fn leading_punctuation_empty_tokens_and_backwards_timestamps_are_tolerated() {
        let words = words_from_tokens(
            &tokens(&[("，", 0), ("", 50), (" 好", 300), (" 的", 250), ("！", 500)]),
            600,
        );
        assert_eq!(texts(&words), ["好", "的！"]);
        assert_eq!((words[0].start_ms, words[0].end_ms), (300, 300));
        assert_eq!((words[1].start_ms, words[1].end_ms), (300, 500));
        assert!(words_from_tokens(&[], 1000).is_empty());
    }

    #[test]
    fn characters_without_markers_still_split_per_cjk_character() {
        // 有的模型输出的中文 token 不带空格
        let words = words_from_tokens(&tokens(&[("你", 0), ("好", 200), ("Rust", 400)]), 900);
        assert_eq!(texts(&words), ["你", "好", "Rust"]);
    }

    #[test]
    fn plain_text_round_trips_through_tokens() {
        for text in [
            "我之前用Rust写过一个推荐系统，效果不错。",
            "This is a tricky question. I'm not sure!",
        ] {
            let plain = plain_tokens(text);
            let timed: Vec<Token<'_>> = plain
                .iter()
                .enumerate()
                .map(|(i, token)| Token {
                    text: token,
                    start_ms: i as u64 * 100,
                })
                .collect();
            assert_eq!(join_words(&words_from_tokens(&timed, 10_000)), text);
        }
    }
}
