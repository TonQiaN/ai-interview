//! 语音的 HTTP 接口：引擎列表、音色管理、参考录音上传、试听。路径和约定见 `docs/architecture/api.md`。
//!
//! 接进服务进程用 [`Deps`] 和 [`router`]。这个模块没有后台任务：合成题目的语音由 `interview-flow` 的任务调
//! [`crate::Synthesizer`]，这里只管音色的增删改查和试听。
//!
//! 参考录音是真人的声音，逐字稿是这段录音的内容：都不进日志，报错信息里也不带。

use std::sync::Arc;

use axum::http::StatusCode;
use interview_audio::Ffmpeg;
use interview_platform::{ApiError, AppContext};
use utoipa_axum::router::OpenApiRouter;

use crate::backend::TtsError;
use crate::engine::{ENGINE_ENV_VAR, EngineId};
use crate::request::RequestError;
use crate::synthesizer::Synthesizer;

mod blob_cache;
mod engines;
mod preview;
mod voices;

pub use blob_cache::BlobAudioCache;
pub use engines::{EngineInfo, EngineList};
pub use preview::PreviewRequest;
pub use voices::{CreateVoice, UpdateVoice, VoiceKind, VoiceView};

/// 语音接口需要的东西。
#[derive(Clone)]
pub struct Deps {
    pub ctx: AppContext,
    /// 语音合成。试听用它，引擎是否可用也问它。
    pub synth: Arc<Synthesizer>,
    /// 上传的参考录音不是 WAV 时，用它转成 WAV。
    pub ffmpeg: Ffmpeg,
    /// 新建音色时默认选哪个引擎（`AI_INTERVIEW_TTS_ENGINE`）。
    pub default_engine: EngineId,
}

impl Deps {
    /// ffmpeg 的路径和默认引擎从环境变量读。
    pub fn from_env(ctx: AppContext, synth: Arc<Synthesizer>) -> Self {
        let default_engine = match std::env::var(ENGINE_ENV_VAR) {
            Ok(value) if !value.trim().is_empty() => value.trim().parse().unwrap_or_else(|error| {
                tracing::warn!(%error, "环境变量 {ENGINE_ENV_VAR} 的值不对，用默认引擎");
                EngineId::DEFAULT
            }),
            _ => EngineId::DEFAULT,
        };
        Self {
            ctx,
            synth,
            ffmpeg: Ffmpeg::from_env(),
            default_engine,
        }
    }
}

/// 语音这条线的全部接口。
pub fn router(deps: Deps) -> OpenApiRouter {
    OpenApiRouter::new()
        .merge(engines::router())
        .merge(voices::router())
        .merge(preview::router())
        .with_state(deps)
}

/// 请求不满足引擎的要求：用户改了就能过，422。
pub(crate) fn request_error(error: &RequestError) -> ApiError {
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "invalid_synthesis_request",
        error.to_string(),
    )
}

/// 把合成的错误换成接口的错误。
///
/// 引擎没开是 503（`tts_engine_unavailable`），合成过程中出错是 502（`tts_error`）。
/// 后端的原始报错只进日志，不返回给前端。
pub(crate) fn tts_error(error: TtsError) -> ApiError {
    match &error {
        TtsError::InvalidRequest(request) => request_error(request),
        TtsError::EngineUnavailable { engine, .. } => ApiError::unavailable(format!(
            "{} 现在用不了，请确认 Mac 上的语音合成服务开着",
            engine.spec().name
        ))
        .with_code("tts_engine_unavailable")
        .with_source(error),
        TtsError::Backend(_) => ApiError::new(
            StatusCode::BAD_GATEWAY,
            "tts_error",
            "语音合成出错了，请稍后再试",
        )
        .with_source(error),
        TtsError::BadAudio(_) => ApiError::new(
            StatusCode::BAD_GATEWAY,
            "tts_error",
            "合成出来的声音不对，请换一句话或者稍后再试",
        )
        .with_source(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_down_is_503_and_backend_details_stay_out_of_the_message() {
        let error = tts_error(TtsError::EngineUnavailable {
            engine: EngineId::Index2,
            reason: "connect to 10.0.0.7:7801 refused".to_owned(),
        });
        assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error.code(), "tts_engine_unavailable");
        assert!(!error.message().contains("10.0.0.7"));

        let error = tts_error(TtsError::Backend("Traceback ...".to_owned()));
        assert_eq!(error.status(), StatusCode::BAD_GATEWAY);
        assert!(!error.message().contains("Traceback"));

        let error = tts_error(TtsError::InvalidRequest(RequestError::EmptyText));
        assert_eq!(error.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error.code(), "invalid_synthesis_request");
    }
}
