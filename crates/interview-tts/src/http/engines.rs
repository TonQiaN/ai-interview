//! 引擎列表：三个 TTS 引擎各自能做什么、现在能不能用。

use axum::extract::State;
use interview_platform::auth::AdminUser;
use interview_platform::http::{ApiResult, Json};
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::Deps;
use crate::engine::{EngineId, Language};

pub(super) fn router() -> OpenApiRouter<Deps> {
    OpenApiRouter::new().routes(routes!(list_engines))
}

/// 一个 TTS 引擎。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EngineInfo {
    pub id: EngineId,
    /// 给人看的名字，比如 `IndexTTS 2`
    pub name: String,
    /// 能说哪些语言
    pub languages: Vec<Language>,
    /// 是不是必须给参考录音（只能克隆，不能用文字设计音色）
    pub requires_reference_audio: bool,
    /// 克隆时是不是必须给参考录音的逐字稿
    pub requires_reference_transcript: bool,
    /// 一次最多合成多少个字。更长的文本会自动切成几段，不用前端管；没有上限时是 null
    pub max_text_chars: Option<u32>,
    /// 模型的许可是不是只允许非商业使用
    pub non_commercial_only: bool,
    /// 能不能用一句文字描述设计音色
    pub supports_voice_design: bool,
    /// 能不能指定情绪
    pub supports_emotion: bool,
    /// 能不能用一句话指导语气
    pub supports_instruction: bool,
    /// 现在能不能用
    pub available: bool,
    /// 不能用的原因
    pub unavailable_reason: Option<String>,
    /// 是不是新建音色时默认选的引擎
    pub is_default: bool,
}

/// 引擎列表。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EngineList {
    pub items: Vec<EngineInfo>,
    /// 合成用的后端：`fake`（输出测试音）或 `worker`（Mac 上的真模型）
    pub backend: String,
    /// 现在合成出来的是不是测试音。是的话试听和面试里听到的都不是真人声
    pub simulated: bool,
}

/// TTS 引擎列表
///
/// 三个引擎各自的能力和现在能不能用。能不能用是当场问合成后端的。
#[utoipa::path(
    get,
    path = "/api/v1/speech/engines",
    tag = "speech",
    responses((status = 200, description = "引擎列表", body = EngineList))
)]
async fn list_engines(State(deps): State<Deps>, _user: AdminUser) -> ApiResult<Json<EngineList>> {
    let backend = deps.synth.backend_name();
    let statuses = deps.synth.status().await;
    let items = EngineId::ALL
        .into_iter()
        .map(|id| {
            let spec = id.spec();
            let status = statuses.iter().find(|status| status.engine == id);
            let available = status.is_some_and(|status| status.ready);
            let unavailable_reason = match status {
                Some(status) if status.ready => None,
                Some(status) => Some(
                    status
                        .detail
                        .clone()
                        .unwrap_or_else(|| "合成后端没有说明原因".to_owned()),
                ),
                None => Some("合成后端没有这个引擎".to_owned()),
            };
            EngineInfo {
                id,
                name: spec.name.to_owned(),
                languages: spec.languages.to_vec(),
                requires_reference_audio: spec.requires_reference_audio,
                requires_reference_transcript: spec.requires_reference_transcript,
                max_text_chars: spec
                    .max_text_chars
                    .map(|chars| u32::try_from(chars).unwrap_or(u32::MAX)),
                non_commercial_only: spec.non_commercial_only,
                supports_voice_design: spec.supports_voice_design,
                supports_emotion: spec.supports_emotion,
                supports_instruction: spec.supports_instruction,
                available,
                unavailable_reason,
                is_default: id == deps.default_engine,
            }
        })
        .collect();
    Ok(Json(EngineList {
        items,
        backend: backend.to_owned(),
        simulated: backend == "fake",
    }))
}
