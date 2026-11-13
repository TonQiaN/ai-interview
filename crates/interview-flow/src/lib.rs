//! 面试场次：一场面试从建好到可以发链接之前的全部事情。
//!
//! - 建面试、改配置、取消、归档
//! - 出题（调 `interview-engine`）和预合成语音（调 `interview-tts`），放在后台任务里跑
//! - 看题、改题、试听
//! - 面试配置模板、配置表单用的默认值和取值范围
//!
//! 接进服务进程的方式见 `interview-server` 的 `compose.rs`。

use std::sync::Arc;

use interview_core::task_kind;
use interview_llm::ChatModel;
use interview_platform::AppContext;
use interview_platform::tasks::TaskRegistry;
use interview_tts::Synthesizer;
use utoipa_axum::router::OpenApiRouter;

pub mod model;
pub mod prepare;
mod routes;
pub mod scrub;
mod store;
mod templates;
pub mod voice;

/// 面试场次模块需要的东西。
#[derive(Clone)]
pub struct Deps {
    pub ctx: AppContext,
    /// 大模型。没配置时是 `None`，出题会失败并把原因写在面试上。
    pub llm: Option<Arc<dyn ChatModel>>,
    /// 语音合成。
    pub synth: Arc<Synthesizer>,
}

/// 面试场次的全部接口。
pub fn router(deps: Deps) -> OpenApiRouter {
    routes::router().merge(templates::router()).with_state(deps)
}

/// 登记后台任务：出题加预合成、只重新合成。
pub fn register_tasks(registry: &mut TaskRegistry, deps: Deps) {
    let prepare_deps = deps.clone();
    registry.register_fn(task_kind::INTERVIEW_PREPARE, move |task| {
        let deps = prepare_deps.clone();
        async move { prepare::run(&deps, &task).await }
    });
    registry.register_fn(task_kind::INTERVIEW_SYNTHESIZE, move |task| {
        let deps = deps.clone();
        async move { prepare::run(&deps, &task).await }
    });
}
