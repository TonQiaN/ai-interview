use async_trait::async_trait;
use interview_audio::Audio;
use serde::Serialize;

use crate::transcript::{Segment, Transcript};

/// 转写时可以调的东西。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TranscribeOptions {
    /// 希望模型优先认出来的词，比如岗位描述和简历里的技术名词、公司名、产品名。
    /// 不是每个后端都支持，不支持的会忽略。
    pub hotwords: Vec<String>,
}

/// 转写失败的原因。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SttError {
    /// 识别后端现在用不了（模型文件没下载、加载失败）。
    #[error("语音识别现在用不了：{0}")]
    Unavailable(String),
    /// 音频本身有问题。
    #[error("音频没法转写：{0}")]
    BadAudio(String),
    /// 识别过程中出错。
    #[error("语音识别出错：{0}")]
    Backend(String),
}

/// 识别后端现在能不能用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SttStatus {
    /// 后端的名字，比如 `fake`、`sherpa`。
    pub backend: String,
    pub ready: bool,
    /// 用的是哪个模型，或者不能用的原因。
    pub detail: Option<String>,
}

/// 增量转写进行到哪了。时间都是相对这次会话第一块音频的开头，毫秒。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct SessionProgress {
    /// 到现在一共喂进来多长的音频。
    pub audio_ms: u64,
    /// 到现在有没有听到过人说话。
    pub heard_speech: bool,
    /// 现在是不是正在说话。
    pub in_speech: bool,
    /// 已经连续安静了多久；正在说话时是 0，还没人说过话时等于 `audio_ms`。
    pub silence_ms: u64,
}

/// 识别后端。
///
/// 两种用法：
///
/// - 整段转写：[`Transcriber::transcribe`]，给一段录好的音频，返回带逐字时间戳的文字。
/// - 边录边转：[`Transcriber::start_session`] 开一个会话，音频来一块喂一块。
///   候选人每说完一小段（停顿一下），这一小段的文字就定下来，答完时只剩最后一小段要转，
///   所以两分钟的回答答完后不用再等几秒。
///
/// 音频可以是任意采样率，后端自己转成模型要的 16 kHz。
/// 没听到人说话不算错，返回空的 [`Transcript`]。
#[async_trait]
pub trait Transcriber: Send + Sync {
    /// 后端现在能不能用，系统状态页会显示。
    async fn status(&self) -> SttStatus;

    /// 开一个边录边转的会话。一个会话对应一段连续的音频（比如候选人的一次回答）。
    async fn start_session(
        &self,
        options: &TranscribeOptions,
    ) -> Result<Box<dyn TranscribeSession>, SttError>;

    /// 整段转写。返回的时间相对这段音频的开头。
    async fn transcribe(
        &self,
        audio: &Audio,
        options: &TranscribeOptions,
    ) -> Result<Transcript, SttError> {
        let mut session = self.start_session(options).await?;
        session.push(audio).await?;
        session.finish().await
    }
}

/// 一次边录边转的会话，由 [`Transcriber::start_session`] 开出来。
///
/// 按顺序调 [`push`](Self::push) 喂音频，最后调 [`finish`](Self::finish) 拿整段结果。
/// 中途丢掉会话就是放弃这次转写，不会留下后台任务。
#[async_trait]
pub trait TranscribeSession: Send {
    /// 喂一块音频，接在之前喂的后面。每一块可以是任意长度、任意采样率。
    ///
    /// 返回这次新定下来的几段文字（多数时候是空的：一段话要等说话人停顿了才定下来）。
    /// 已经返回过的段不会再变。
    async fn push(&mut self, audio: &Audio) -> Result<Vec<Segment>, SttError>;

    /// 进行到哪了：喂了多长、现在是不是在说话、安静了多久。
    fn progress(&self) -> SessionProgress;

    /// 到现在已经定下来的文字。还没说完的那一小段不在里面。
    fn transcript(&self) -> Transcript;

    /// 音频到此为止：把剩下的转完，返回整段的结果（包含之前 `push` 返回过的所有段）。
    async fn finish(self: Box<Self>) -> Result<Transcript, SttError>;
}
