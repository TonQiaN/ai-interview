//! 装配：把各条线的模块接进服务进程。**各线接入自己的 crate 只改这个文件。**
//!
//! 每个带 HTTP 的 crate 导出三样东西（约定见 `docs/architecture/README.md`）：
//!
//! ```ignore
//! pub struct Deps { pub ctx: AppContext, /* 自己需要的其他东西 */ }
//! pub fn router(deps: Deps) -> OpenApiRouter;
//! pub fn register_tasks(registry: &mut TaskRegistry, deps: Deps);
//! ```
//!
//! 接入时在 [`compose`] 里加三行：`api = api.merge(xxx::router(deps.clone()))`、
//! `xxx::register_tasks(&mut tasks, deps)`，有外部依赖的再 `probes.register(...)`。
//! 同时在本 crate 的 `Cargo.toml` 和根 `Cargo.toml` 的 `[workspace.dependencies]` 里加上这个 crate。

use std::sync::Arc;

use async_trait::async_trait;
use interview_llm::{ChatModel, DeepSeekClient};
use interview_platform::AppContext;
use interview_platform::status::{ComponentStatus, DatabaseProbe, StatusProbe, StatusRegistry};
use interview_platform::tasks::TaskRegistry;
use utoipa_axum::router::OpenApiRouter;

/// 多个模块共用、又不属于平台的东西。
#[derive(Clone)]
pub struct Shared {
    /// 大模型。没配密钥时是 `None`：服务照常启动，用到大模型的功能返回 503。
    pub llm: Option<Arc<dyn ChatModel>>,
}

impl Shared {
    /// 按环境变量创建。
    pub fn from_env() -> Self {
        let llm = match DeepSeekClient::from_env() {
            Ok(client) => Some(Arc::new(client) as Arc<dyn ChatModel>),
            Err(err) => {
                tracing::warn!(reason = %err, "大模型没配置，出题、简历解析、分析报告暂时不可用");
                None
            }
        };
        Self { llm }
    }

    /// 什么外部依赖都没有。生成 OpenAPI 文档和测试时用。
    pub fn none() -> Self {
        Self { llm: None }
    }
}

/// 装配的结果。
pub struct Composed {
    /// 所有模块的接口，带 OpenAPI 描述
    pub api: OpenApiRouter,
    /// 所有模块的后台任务
    pub tasks: TaskRegistry,
}

/// 把各模块装在一起。
pub fn compose(ctx: &AppContext, shared: &Shared) -> Composed {
    // `tasks` 现在还没有模块登记，接入第一个带后台任务的 crate 时去掉这个 allow
    #[allow(unused_mut, reason = "各线接入后会往里登记任务")]
    let mut tasks = TaskRegistry::new();
    let mut probes = StatusRegistry::new();
    probes.register(DatabaseProbe(ctx.db.clone()));
    probes.register(LlmProbe {
        configured: shared.llm.is_some(),
    });

    let mut api = OpenApiRouter::new();

    // ── ② 岗位和候选人：interview-talent、interview-raas ──

    // ── ③ AI 引擎：interview-engine 是纯库，由 interview-flow 调用，这里不用接 ──

    // ── ④ 语音：interview-tts（音色管理、合成）、interview-stt ──

    // ── ⑤ 候选人面试间：interview-room ──

    // ── ① 面试场次：interview-flow ──

    // ── ① 平台：登录、设置、系统状态。放最后，因为它要拿到登记完的探针 ──
    api = api.merge(interview_platform::routes::router(
        interview_platform::routes::Deps {
            ctx: ctx.clone(),
            probes: Arc::new(probes),
        },
    ));

    Composed { api, tasks }
}

/// 大模型有没有配置。只看有没有密钥，不真的调用（调用要花钱）。
struct LlmProbe {
    configured: bool,
}

#[async_trait]
impl StatusProbe for LlmProbe {
    fn name(&self) -> &'static str {
        "llm"
    }

    async fn check(&self) -> ComponentStatus {
        if self.configured {
            ComponentStatus::ok(self.name())
        } else {
            ComponentStatus::not_configured(self.name(), "没有设置环境变量 DEEPSEEK_API_KEY")
        }
    }
}
