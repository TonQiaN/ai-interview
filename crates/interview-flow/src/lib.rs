//! 面试场次：一场面试除了候选人在面试间里的那一段，前前后后的全部事情。
//!
//! 面试前：
//! - 建面试、改配置、取消、归档
//! - 出题（调 `interview-engine`）和预合成语音（调 `interview-tts`），放在后台任务里跑
//! - 看题、改题、试听
//! - 面试配置模板、配置表单用的默认值和取值范围
//!
//! 面试后：
//! - 重新转写候选人的回答（调 `interview-stt`），生成分析报告（调 `interview-report`），都在后台任务里跑
//! - 对话记录、分析报告、HR 的结论
//!
//! 接进服务进程的方式见 `interview-server` 的 `compose.rs`。

use std::sync::Arc;

use interview_core::task_kind;
use interview_llm::ChatModel;
use interview_platform::AppContext;
use interview_platform::tasks::TaskRegistry;
use interview_stt::Transcriber;
use interview_tts::Synthesizer;
use utoipa_axum::router::OpenApiRouter;

mod context;
pub mod finalize;
pub mod model;
pub mod prepare;
mod record;
pub mod report;
mod review;
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
    /// 语音识别。没有时是 `None`，面试后不重新转写，报告用面试中转写的文字。
    pub stt: Option<Arc<dyn Transcriber>>,
}

/// 面试场次的全部接口。
pub fn router(deps: Deps) -> OpenApiRouter {
    routes::router()
        .merge(review::router())
        .merge(templates::router())
        .with_state(deps)
}

/// 登记后台任务：出题加预合成、只重新合成、面试后的收尾、生成报告。
pub fn register_tasks(registry: &mut TaskRegistry, deps: Deps) {
    let task_deps = deps.clone();
    registry.register_fn(task_kind::INTERVIEW_PREPARE, move |task| {
        let deps = task_deps.clone();
        async move { prepare::run(&deps, &task).await }
    });
    let task_deps = deps.clone();
    registry.register_fn(task_kind::INTERVIEW_SYNTHESIZE, move |task| {
        let deps = task_deps.clone();
        async move { prepare::run(&deps, &task).await }
    });
    let task_deps = deps.clone();
    registry.register_fn(task_kind::INTERVIEW_FINALIZE, move |task| {
        let deps = task_deps.clone();
        async move { finalize::run(&deps, &task).await }
    });
    registry.register_fn(task_kind::REPORT_GENERATE, move |task| {
        let deps = deps.clone();
        async move { report::run(&deps, &task).await }
    });
}
