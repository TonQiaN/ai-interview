//! 合成前把引擎读不准的多音字换成带拼音的写法。
//!
//! IndexTTS 2 的文本里可以写「大写拼音加声调数字」来指定一个字怎么读（「去CHONG2」），
//! 实测见 `tts/worker/README.md` 的「用拼音纠正多音字」。这里只换交给引擎的那份文字：
//! 请求里的 `text` 不变，字幕和对话记录里看不到拼音。
//!
//! 表里只收实测读错、标了拼音以后读对的词。面试里再发现读错的多音字，记在 issue #180 里，
//! 在 Mac 上试过以后加进来。夹在中文里的英文词拼音帮不上。

use std::borrow::Cow;

use crate::engine::EngineId;

/// 一个读不准的词。
struct Fix {
    /// 原来的写法。
    word: &'static str,
    /// 带拼音的写法。
    spoken: &'static str,
    /// 后面紧跟着这些字时不换：那是另一个词，读音不一样。
    /// 「过去重要的」里也有「去重」两个字，「重」读 zhòng。
    /// 漏换只是和没有这张表时一样，换错会把本来读对的读错，所以宁可多列。
    unless_followed_by: &'static str,
}

const INDEX2: &[Fix] = &[Fix {
    word: "去重",
    spoken: "去CHONG2",
    unless_followed_by: "要点视大量心任型度伤病罚责金镇担负力压用创击托症",
}];

fn fixes_for(engine: EngineId) -> &'static [Fix] {
    match engine {
        EngineId::Index2 => INDEX2,
        // IndexTTS 2.5 和 BreezeTTS2 认不认拼音没有试过，不换
        EngineId::Index25 | EngineId::Breeze2 => &[],
    }
}

/// 交给引擎的文字。大多数时候就是原文。
pub(crate) fn spoken_text(engine: EngineId, text: &str) -> Cow<'_, str> {
    let mut spoken = Cow::Borrowed(text);
    for fix in fixes_for(engine) {
        if spoken.contains(fix.word) {
            spoken = Cow::Owned(apply(fix, &spoken));
        }
    }
    spoken
}

fn apply(fix: &Fix, text: &str) -> String {
    let mut out = String::with_capacity(text.len() + fix.spoken.len());
    let mut rest = text;
    while let Some(at) = rest.find(fix.word) {
        let after = &rest[at + fix.word.len()..];
        let other_word = after
            .chars()
            .next()
            .is_some_and(|next| fix.unless_followed_by.contains(next));
        out.push_str(&rest[..at]);
        out.push_str(if other_word { fix.word } else { fix.spoken });
        rest = after;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index2_gets_pinyin_for_words_it_misreads() {
        assert_eq!(
            spoken_text(EngineId::Index2, "数据入库之前要先去重。"),
            "数据入库之前要先去CHONG2。"
        );
        assert_eq!(
            spoken_text(EngineId::Index2, "去重是在写入前做，还是读的时候再去重？"),
            "去CHONG2是在写入前做，还是读的时候再去CHONG2？"
        );
    }

    #[test]
    fn text_without_such_words_is_left_alone() {
        let text = "请介绍一下你最近做的项目。";
        assert!(matches!(
            spoken_text(EngineId::Index2, text),
            Cow::Borrowed(same) if same == text
        ));
    }

    #[test]
    fn the_same_characters_in_another_word_are_left_alone() {
        for text in ["你过去重点负责哪一块？", "过去重要的项目里，哪个最难？"]
        {
            assert_eq!(spoken_text(EngineId::Index2, text), text);
        }
        // 一句里两种都有：只换该换的那个
        assert_eq!(
            spoken_text(EngineId::Index2, "过去重点做的是消息去重。"),
            "过去重点做的是消息去CHONG2。"
        );
    }

    #[test]
    fn engines_that_were_not_tested_get_the_original_text() {
        let text = "数据入库之前要先去重。";
        assert_eq!(spoken_text(EngineId::Index25, text), text);
        assert_eq!(spoken_text(EngineId::Breeze2, text), text);
    }
}
