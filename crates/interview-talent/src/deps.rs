//! 岗位和候选人这条线对外的三样东西之一：[`Deps`]。另外两样是 [`crate::router`] 和
//! [`crate::register_tasks`]。

use std::sync::Arc;

use interview_llm::ChatModel;
use interview_platform::{ApiError, ApiResult, AppContext};

use crate::render::PdfRenderer;
use crate::settings::TalentAiSettings;

/// 接口和后台任务需要的东西。克隆很便宜。
#[derive(Clone)]
pub struct Deps {
    pub ctx: AppContext,
    /// 大模型。没配置时是 `None`：增删改查照常用，要大模型的功能返回 503。
    pub llm: Option<Arc<dyn ChatModel>>,
    /// 读简历、整理岗位、写画像时的思考档位。
    pub ai: TalentAiSettings,
    /// 把 PDF 简历渲染成图片的设置。
    pub renderer: PdfRenderer,
}

impl Deps {
    /// 思考档位和渲染设置都用默认值。
    pub fn new(ctx: AppContext, llm: Option<Arc<dyn ChatModel>>) -> Self {
        Self {
            ctx,
            llm,
            ai: TalentAiSettings::default(),
            renderer: PdfRenderer::default(),
        }
    }

    /// 思考档位从环境变量读（`AI_INTERVIEW_TALENT_THINKING`、`AI_INTERVIEW_TALENT_PROFILE_THINKING`）。
    /// 写错了不让服务起不来：打一条警告，用默认值。
    pub fn from_env(ctx: AppContext, llm: Option<Arc<dyn ChatModel>>) -> Self {
        let ai = TalentAiSettings::from_env().unwrap_or_else(|error| {
            tracing::warn!(%error, "岗位和候选人的大模型设置有误，改用默认值");
            TalentAiSettings::default()
        });
        Self {
            ai,
            ..Self::new(ctx, llm)
        }
    }

    /// 要用大模型的接口先调这个：没配置就返回 503。
    pub(crate) fn require_llm(&self) -> ApiResult<Arc<dyn ChatModel>> {
        self.llm.clone().ok_or_else(|| {
            ApiError::unavailable("大模型还没有配置，这个功能暂时用不了")
                .with_code("llm_unavailable")
        })
    }
}

impl std::fmt::Debug for Deps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Deps")
            .field("llm", &self.llm.is_some())
            .field("ai", &self.ai)
            .field("renderer", &self.renderer)
            .finish_non_exhaustive()
    }
}
