//! 试听：用某个音色把一句话合成出来，直接返回声音。

use axum::body::Body;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::Response;
use interview_platform::auth::AdminUser;
use interview_platform::http::{ApiError, ApiResult, ErrorBody, Json};
use serde::Deserialize;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use super::voices::{LoadedVoice, check_settings, load_for_synthesis};
use super::{Deps, tts_error};
use crate::engine::{EngineId, Language};
use crate::request::{Style, SynthesisOptions, SynthesisRequest, Voice, VoiceSettings};

/// 试听的文本最多多少个字。试听是同步等结果的，太长会等很久。
const MAX_PREVIEW_CHARS: usize = 300;
const MAX_DESCRIPTION_CHARS: usize = 200;

pub(super) fn router() -> OpenApiRouter<Deps> {
    OpenApiRouter::new().routes(routes!(preview))
}

/// 试听请求。
///
/// 用哪个声音，三选一：`voice_id` 指定一个已有的音色；`description` 临时用一句文字设计一个（不用先建音色）；
/// 两个都不填用默认音色。
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct PreviewRequest {
    /// 要念的话，最多 300 个字
    pub text: String,
    /// 用哪个音色
    pub voice_id: Option<Uuid>,
    /// 临时设计一个音色的文字描述，比如「三十岁左右的女声，沉稳、亲切」。和 `voice_id` 只能填一个
    pub description: Option<String>,
    /// 换一个引擎听听。不填用音色自己的引擎；配合 `description` 时不填是 BreezeTTS2
    pub engine: Option<EngineId>,
    /// 不填用音色自己的设置
    pub language: Option<Language>,
    /// 不填用音色自己的设置。可以在保存之前先听听改了情绪、语气指令以后的效果
    pub style: Option<Style>,
    /// 不填用音色自己的设置
    pub options: Option<SynthesisOptions>,
}

/// 试听
///
/// 同步合成，直接返回 WAV（16 位单声道）。真模型合成一句话要几秒到十几秒，前端要给等待提示。
/// 合成后端是测试音（见引擎列表的 `simulated`）时，听到的是一段提示音，不是人声。
#[utoipa::path(
    post,
    path = "/api/v1/speech/preview",
    tag = "speech",
    request_body = PreviewRequest,
    responses(
        (
            status = 200,
            description = "合成出来的声音",
            content_type = "audio/wav",
            headers(
                ("x-audio-duration-ms" = u64, description = "这段声音多少毫秒"),
                ("x-tts-engine" = String, description = "实际用的引擎"),
                ("x-tts-backend" = String, description = "合成后端：fake 是测试音，worker 是真模型"),
                ("x-tts-from-cache" = bool, description = "是不是以前合成过、直接拿的缓存"),
            ),
        ),
        (status = 404, description = "没有这个音色", body = ErrorBody),
        (status = 422, description = "文本、音色或设置不合要求（比如音色还没上传参考录音，或者引擎不支持这种音色）", body = ErrorBody),
        (status = 502, description = "合成出错", body = ErrorBody),
        (status = 503, description = "这个引擎现在用不了", body = ErrorBody),
    )
)]
async fn preview(
    State(deps): State<Deps>,
    _user: AdminUser,
    Json(request): Json<PreviewRequest>,
) -> ApiResult<Response> {
    let text = request.text.trim();
    if text.is_empty() {
        return Err(ApiError::invalid_field("text", "要填一句试听的话"));
    }
    if text.chars().count() > MAX_PREVIEW_CHARS {
        return Err(ApiError::invalid_field(
            "text",
            format!("试听的话最多 {MAX_PREVIEW_CHARS} 个字"),
        ));
    }
    let description = request
        .description
        .as_deref()
        .map(str::trim)
        .filter(|description| !description.is_empty());

    let LoadedVoice {
        engine,
        voice,
        settings,
    } = match (request.voice_id, description) {
        (Some(_), Some(_)) => {
            return Err(ApiError::invalid_field(
                "description",
                "已经选了音色，就不用再填音色描述了，两个只能填一个",
            ));
        }
        (Some(id), None) => load_for_synthesis(&deps, Some(id))
            .await?
            .ok_or_else(|| ApiError::not_found("音色"))?,
        (None, Some(description)) => {
            if description.chars().count() > MAX_DESCRIPTION_CHARS {
                return Err(ApiError::invalid_field(
                    "description",
                    format!("最多 {MAX_DESCRIPTION_CHARS} 个字"),
                ));
            }
            LoadedVoice {
                engine: EngineId::Breeze2,
                voice: Voice::Design {
                    description: description.to_owned(),
                },
                settings: VoiceSettings::default(),
            }
        }
        (None, None) => load_for_synthesis(&deps, None).await?.ok_or_else(|| {
            ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "no_default_voice",
                "还没有默认音色。请选一个音色，或者填一句音色描述",
            )
        })?,
    };

    let settings = VoiceSettings {
        language: request.language.unwrap_or(settings.language),
        style: request.style.unwrap_or(settings.style),
        options: request.options.unwrap_or(settings.options),
    };
    check_settings(&settings)?;
    let mut synthesis = SynthesisRequest::new(
        request.engine.unwrap_or(engine),
        text,
        settings.language,
        voice,
    );
    synthesis.style = settings.style;
    synthesis.options = settings.options;

    let result = deps.synth.synthesize(&synthesis).await.map_err(tts_error)?;
    let duration_ms = result.duration_ms();
    Response::builder()
        .header(header::CONTENT_TYPE, "audio/wav")
        .header(header::CONTENT_LENGTH, result.wav.len())
        // 试听的结果和当时的音色设置有关，不让浏览器缓存
        .header(header::CACHE_CONTROL, "no-store")
        .header("x-audio-duration-ms", duration_ms)
        .header("x-tts-engine", synthesis.engine.as_str())
        .header("x-tts-backend", deps.synth.backend_name())
        .header(
            "x-tts-from-cache",
            if result.from_cache { "true" } else { "false" },
        )
        .body(Body::from(result.wav))
        .map_err(ApiError::internal)
}
