use serde::{Deserialize, Serialize};

/// 一个词和它的起止时间。中文一个字是一个词，英文一个单词是一个词，标点跟在前一个词后面。
///
/// 序列化成 `{"w": "字", "s": 1200, "e": 1350}`，时间是毫秒。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Word {
    #[serde(rename = "w")]
    pub text: String,
    #[serde(rename = "s")]
    pub start_ms: u64,
    #[serde(rename = "e")]
    pub end_ms: u64,
}

/// 连续说的一段话，中间没有长停顿。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub words: Vec<Word>,
    /// 模型对这一段的把握，0 到 1。给不出来的模型是 `None`。
    pub confidence: Option<f32>,
}

/// 一段音频的转写结果。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Transcript {
    /// 按时间顺序的各段。没听到人说话时是空的。
    pub segments: Vec<Segment>,
    /// 被转写的音频有多长。
    pub audio_duration_ms: u64,
}

impl Transcript {
    /// 有没有听到人说话。
    pub fn is_empty(&self) -> bool {
        self.segments
            .iter()
            .all(|segment| segment.text.trim().is_empty())
    }

    /// 全文：各段的文字按顺序接起来。
    pub fn text(&self) -> String {
        let mut text = String::new();
        for segment in &self.segments {
            let piece = segment.text.trim();
            if piece.is_empty() {
                continue;
            }
            if needs_space(text.chars().next_back(), piece.chars().next()) {
                text.push(' ');
            }
            text.push_str(piece);
        }
        text
    }

    /// 所有词，按时间顺序。
    pub fn words(&self) -> impl Iterator<Item = &Word> {
        self.segments
            .iter()
            .flat_map(|segment| segment.words.iter())
    }

    /// 第一个词开始的时间。
    pub fn speech_start_ms(&self) -> Option<u64> {
        self.segments.first().map(|segment| segment.start_ms)
    }

    /// 最后一个词结束的时间。
    pub fn speech_end_ms(&self) -> Option<u64> {
        self.segments.last().map(|segment| segment.end_ms)
    }

    /// 把所有时间往后挪 `offset_ms`。
    ///
    /// 转写出来的时间相对这段音频的开头；面试里统一用相对面试开始的毫秒数，
    /// 所以写库之前要加上这段音频在面试里的起点。
    pub fn shifted(mut self, offset_ms: u64) -> Self {
        for segment in &mut self.segments {
            segment.start_ms += offset_ms;
            segment.end_ms += offset_ms;
            for word in &mut segment.words {
                word.start_ms += offset_ms;
                word.end_ms += offset_ms;
            }
        }
        self
    }
}

/// 两段文字接在一起时中间要不要空格：两边都是字母或数字（英文单词）才要。
pub(crate) fn needs_space(before: Option<char>, after: Option<char>) -> bool {
    matches!((before, after), (Some(a), Some(b)) if is_latin_word_char(a) && is_latin_word_char(b))
        || matches!((before, after), (Some(a), Some(b)) if is_latin_punctuation(a) && is_latin_word_char(b))
}

fn is_latin_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '\''
}

/// 英文句子里后面要跟空格的标点。
fn is_latin_punctuation(c: char) -> bool {
    matches!(c, ',' | '.' | '?' | '!' | ';' | ':')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, start_ms: u64, end_ms: u64) -> Word {
        Word {
            text: text.to_owned(),
            start_ms,
            end_ms,
        }
    }

    fn segment(text: &str, start_ms: u64, end_ms: u64) -> Segment {
        Segment {
            start_ms,
            end_ms,
            text: text.to_owned(),
            words: vec![word(text, start_ms, end_ms)],
            confidence: None,
        }
    }

    #[test]
    fn word_serializes_in_the_interview_turns_format() {
        let json = serde_json::to_string(&word("字", 1200, 1350)).unwrap();
        assert_eq!(json, r#"{"w":"字","s":1200,"e":1350}"#);
        assert_eq!(
            serde_json::from_str::<Word>(&json).unwrap(),
            word("字", 1200, 1350)
        );
    }

    #[test]
    fn text_joins_segments_with_a_space_only_between_latin_words() {
        let transcript = Transcript {
            segments: vec![
                segment("我之前做过", 0, 900),
                segment("推荐系统。", 1500, 2400),
                segment("It was built with", 3000, 4000),
                segment("Rust.", 4500, 4900),
                segment("  ", 5000, 5100),
                segment("Thanks", 5200, 5600),
            ],
            audio_duration_ms: 6000,
        };
        assert_eq!(
            transcript.text(),
            "我之前做过推荐系统。It was built with Rust. Thanks"
        );
        assert!(!transcript.is_empty());
        assert_eq!(transcript.speech_start_ms(), Some(0));
        assert_eq!(transcript.speech_end_ms(), Some(5600));
    }

    #[test]
    fn an_empty_transcript_has_no_text() {
        let transcript = Transcript {
            segments: vec![],
            audio_duration_ms: 3000,
        };
        assert!(transcript.is_empty());
        assert_eq!(transcript.text(), "");
        assert_eq!(transcript.speech_start_ms(), None);
    }

    #[test]
    fn shifted_moves_every_timestamp_onto_the_interview_timeline() {
        let transcript = Transcript {
            segments: vec![segment("好的", 100, 600)],
            audio_duration_ms: 1000,
        }
        .shifted(60_000);
        assert_eq!(transcript.segments[0].start_ms, 60_100);
        assert_eq!(transcript.segments[0].end_ms, 60_600);
        assert_eq!(
            transcript.segments[0].words[0],
            word("好的", 60_100, 60_600)
        );
        // 音频本身的长度不变
        assert_eq!(transcript.audio_duration_ms, 1000);
    }
}
