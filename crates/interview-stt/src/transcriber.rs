use async_trait::async_trait;
use interview_audio::Audio;
use serde::Serialize;

use crate::transcript::Transcript;

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

/// 整段转写：给一段录好的音频，返回带逐字时间戳的文字。
///
/// 音频可以是任意采样率，后端自己转成模型要的 16 kHz。
/// 返回的时间相对这段音频的开头。没听到人说话不算错，返回空的 [`Transcript`]。
#[async_trait]
pub trait Transcriber: Send + Sync {
    /// 后端现在能不能用，系统状态页会显示。
    async fn status(&self) -> SttStatus;

    async fn transcribe(
        &self,
        audio: &Audio,
        options: &TranscribeOptions,
    ) -> Result<Transcript, SttError>;
}
