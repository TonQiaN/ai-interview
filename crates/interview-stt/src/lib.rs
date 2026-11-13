//! AI 面试的语音识别（STT）。
//!
//! 把候选人的回答转成文字，带逐字的时间戳。面试后的分析报告靠时间戳把一句话对回录像的位置。
//!
//! - [`Transcriber`]：整段转写的接口。输入一段 [`interview_audio::Audio`]，输出 [`Transcript`]。
//! - [`Transcript`]、[`Segment`]、[`Word`]：转写结果。时间都是相对这段音频开头的毫秒数，
//!   写进 `interview_turns` 之前用 [`Transcript::shifted`] 换到面试的时间轴上。
//!   [`Word`] 序列化成 `{"w": "字", "s": 1200, "e": 1350}`，就是 `interview_turns.words` 的格式。
//! - [`words_from_tokens`]：把识别模型输出的 token 和时间戳拼成词，各个模型后端共用。
//! - [`FakeTranscriber`]：不需要模型的假后端，云端和 CI 用。
//!
//! 候选人说的话是个人信息：这个 crate 不把转写文字写进日志。

mod fake;
mod transcriber;
mod transcript;
mod words;

pub use fake::FakeTranscriber;
pub use transcriber::{SttError, SttStatus, TranscribeOptions, Transcriber};
pub use transcript::{Segment, Transcript, Word};
pub use words::{Token, join_words, words_from_tokens};
