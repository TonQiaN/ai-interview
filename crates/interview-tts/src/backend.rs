use std::pin::Pin;

use async_trait::async_trait;
use futures_util::Stream;
use futures_util::stream;
use interview_audio::Audio;
use serde::Serialize;

use crate::engine::EngineId;
use crate::request::{RequestError, SynthesisRequest};

/// 合成失败的原因。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TtsError {
    /// 请求本身不满足引擎的要求，重试没用。
    #[error(transparent)]
    InvalidRequest(#[from] RequestError),
    /// 引擎现在用不了（worker 没开、模型没加载），过一会儿可能恢复。
    #[error("{engine} 现在用不了：{reason}")]
    EngineUnavailable { engine: EngineId, reason: String },
    /// 后端在合成过程中出错。
    #[error("TTS 后端出错：{0}")]
    Backend(String),
    /// 后端返回了音频，但音频明显不对（空的、全是静音、短得不合理）。
    #[error("合成出来的音频不对：{0}")]
    BadAudio(String),
}

/// 一个引擎现在能不能用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EngineStatus {
    pub engine: EngineId,
    pub ready: bool,
    /// 不能用的原因，或者后端想补充的说明。
    pub detail: Option<String>,
}

/// 分块到达的音频。每一块都是一段完整可播的 [`Audio`]，按顺序接起来就是整段。
pub type AudioStream = Pin<Box<dyn Stream<Item = Result<Audio, TtsError>> + Send>>;

/// 真正产生声音的后端。
///
/// 后端收到的请求已经检查过，文本也已经切到引擎的单段上限以内，所以后端只管合成这一段。
/// 目前有两个实现：[`crate::FakeBackend`]（不需要模型，云端和 CI 用）和接 Mac 上常驻服务的 worker 后端。
#[async_trait]
pub trait TtsBackend: Send + Sync {
    /// 后端的名字，出现在日志和系统状态里。
    fn name(&self) -> &'static str;

    /// 三个引擎各自能不能用。
    async fn status(&self) -> Vec<EngineStatus>;

    /// 合成一整段。
    async fn synthesize(&self, request: &SynthesisRequest) -> Result<Audio, TtsError>;

    /// 边合成边返回。不支持流式的后端用默认实现：整段合成完再作为唯一的一块返回。
    async fn synthesize_stream(&self, request: &SynthesisRequest) -> Result<AudioStream, TtsError> {
        let audio = self.synthesize(request).await?;
        Ok(Box::pin(stream::once(async move { Ok(audio) })))
    }
}
