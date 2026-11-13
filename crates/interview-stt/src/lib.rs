//! AI 面试的语音识别（STT）。
//!
//! 把候选人的回答转成文字，带标点和逐字的时间戳。面试后的分析报告靠时间戳把一句话对回录像的位置。
//!
//! - [`Transcriber`]：识别后端的接口。整段转写用 [`Transcriber::transcribe`]；
//!   边录边转用 [`Transcriber::start_session`] 开一个 [`TranscribeSession`]，音频来一块喂一块，
//!   候选人每说完一句（停顿半秒多）这一句的文字就定下来，答完时不用再等整段转写。
//! - [`Transcript`]、[`Segment`]、[`Word`]：转写结果。时间都是相对这段音频开头的毫秒数，
//!   写进 `interview_turns` 之前用 [`Transcript::shifted`] 换到面试的时间轴上。
//!   [`Word`] 序列化成 `{"w": "字", "s": 1200, "e": 1350}`，就是 `interview_turns.words` 的格式。
//! - [`SttConfig`]：从环境变量决定用哪个后端，[`SttConfig::build`] 把它建出来。
//! - `SherpaTranscriber`：真的后端，sherpa-onnx 在进程内跑 X-ASR 模型，CPU 就够，不需要显卡。
//!   编译时要打开 `sherpa` 特性，模型用 `crates/interview-stt/scripts/download-models.sh` 下载。
//!   选型和实测数据见 `docs/speech/stt.md`。
//! - [`FakeTranscriber`]：不需要模型的假后端，云端和 CI 用。
//! - [`words_from_tokens`]：把识别模型输出的 token 和时间戳拼成词，各个模型后端共用。
//!
//! 候选人说的话是个人信息：这个 crate 不把转写文字写进日志。

mod config;
mod fake;
#[cfg(feature = "sherpa")]
mod sherpa;
mod transcriber;
mod transcript;
mod words;

pub use config::{BACKEND_ENV_VAR, MODEL_DIR_ENV_VAR, SttBackend, SttConfig, THREADS_ENV_VAR};
pub use fake::FakeTranscriber;
#[cfg(feature = "sherpa")]
pub use sherpa::{ASR_MODEL_DIR_NAME, SherpaConfig, SherpaTranscriber, VAD_MODEL_FILE_NAME};
pub use transcriber::{
    SessionProgress, SttError, SttStatus, TranscribeOptions, TranscribeSession, Transcriber,
};
pub use transcript::{Segment, Transcript, Word};
pub use words::{Token, join_words, words_from_tokens};
