//! 岗位和候选人这条线对外的三样东西之一：[`Deps`]。另外两样是 [`crate::router`] 和
//! [`crate::register_tasks`]。

use std::sync::Arc;

use interview_llm::ChatModel;
use interview_platform::{ApiError, ApiResult, AppContext};
use interview_raas::{RaasClient, RaasConfig, RaasError};

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
    /// RAAS 的只读客户端。没配置时是 `None`：从 RAAS 导入的接口返回 503，别的照常用。
    pub raas: Option<Arc<RaasClient>>,
}

impl Deps {
    /// 思考档位和渲染设置都用默认值。
    pub fn new(ctx: AppContext, llm: Option<Arc<dyn ChatModel>>) -> Self {
        Self {
            ctx,
            llm,
            ai: TalentAiSettings::default(),
            renderer: PdfRenderer::default(),
            raas: None,
        }
    }

    /// 接上 RAAS。
    #[must_use]
    pub fn with_raas(mut self, raas: Arc<RaasClient>) -> Self {
        self.raas = Some(raas);
        self
    }

    /// 思考档位从环境变量读（`AI_INTERVIEW_TALENT_THINKING`、`AI_INTERVIEW_TALENT_PROFILE_THINKING`）。
    /// RAAS 的地址和账号也从环境变量读（`RAAS_BASE_URL`、`RAAS_ADMIN_USERNAME`、`RAAS_ADMIN_PASSWORD`），
    /// 没配就不接。
    /// 写错了不让服务起不来：打一条警告，用默认值。
    pub fn from_env(ctx: AppContext, llm: Option<Arc<dyn ChatModel>>) -> Self {
        let ai = TalentAiSettings::from_env().unwrap_or_else(|error| {
            tracing::warn!(%error, "岗位和候选人的大模型设置有误，改用默认值");
            TalentAiSettings::default()
        });
        Self {
            ai,
            raas: raas_from_env(),
            ..Self::new(ctx, llm)
        }
    }

    /// 要连 RAAS 的接口先调这个：没配置就返回 503。
    pub(crate) fn require_raas(&self) -> ApiResult<Arc<RaasClient>> {
        self.raas.clone().ok_or_else(|| {
            ApiError::unavailable("还没有配置 RAAS 的地址和账号，这个功能暂时用不了")
                .with_code("raas_unavailable")
        })
    }

    /// 要用大模型的接口先调这个：没配置就返回 503。
    pub(crate) fn require_llm(&self) -> ApiResult<Arc<dyn ChatModel>> {
        self.llm.clone().ok_or_else(|| {
            ApiError::unavailable("大模型还没有配置，这个功能暂时用不了")
                .with_code("llm_unavailable")
        })
    }
}

/// 环境变量齐了就建一个 RAAS 客户端。这里不联网，第一次用到时才登录。
fn raas_from_env() -> Option<Arc<RaasClient>> {
    let config = match RaasConfig::from_env() {
        Ok(config) => config,
        // 没配是正常情况，不用提醒
        Err(RaasError::MissingConfig(_)) => return None,
        Err(error) => {
            tracing::warn!(%error, "RAAS 的配置有误，从 RAAS 导入的功能不可用");
            return None;
        }
    };
    match RaasClient::new(config) {
        Ok(client) => Some(Arc::new(client)),
        Err(error) => {
            tracing::warn!(%error, "RAAS 的配置有误，从 RAAS 导入的功能不可用");
            None
        }
    }
}

impl std::fmt::Debug for Deps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Deps")
            .field("llm", &self.llm.is_some())
            .field("ai", &self.ai)
            .field("renderer", &self.renderer)
            .field("raas", &self.raas.is_some())
            .finish_non_exhaustive()
    }
}
