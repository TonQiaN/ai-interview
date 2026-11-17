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

use interview_audio::Ffmpeg;
use interview_core::task_kind;
use interview_llm::ChatModel;
use interview_platform::AppContext;
use interview_platform::tasks::TaskRegistry;
use interview_stt::Transcriber;
use interview_tts::Synthesizer;
use utoipa_axum::router::OpenApiRouter;

mod answer_audio;
pub mod comparison;
mod context;
pub mod finalize;
pub mod model;
pub mod prepare;
mod purge;
mod record;
pub mod report;
mod review;
mod routes;
pub mod scrub;
mod store;
mod templates;
mod video_check;
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
    /// ffmpeg。面试后重新转写时，用它从录像里取每一段回答的声音。这台机器上没装也能建，
    /// 用到时取不出声音，报告用面试中转写的文字。
    pub ffmpeg: Ffmpeg,
}

/// 面试场次的全部接口。
pub fn router(deps: Deps) -> OpenApiRouter {
    routes::router()
        .merge(review::router())
        .merge(comparison::router())
        .merge(templates::router())
        .with_state(deps)
}

/// 登记后台任务：出题加预合成、只重新合成、面试后的收尾、生成报告、候选人对比，
/// 以及它们没能自己收尾时替它们收尾的函数。
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
    let task_deps = deps.clone();
    registry.register_fn(task_kind::REPORT_GENERATE, move |task| {
        let deps = task_deps.clone();
        async move { report::run(&deps, &task).await }
    });
    let task_deps = deps.clone();
    registry.register_fn(task_kind::POSITION_COMPARE, move |task| {
        let deps = task_deps.clone();
        async move { comparison::run(&deps, &task).await }
    });

    // 处理函数没能自己收尾的（崩溃了，或者进程中断、次数也用完了），替它把「正在做」改成失败，
    // 不然 HR 那边会一直等着、也没法重来。收尾的面试后整理（finalize）不用：它没有「正在做」的状态，
    // 没做完 HR 照样能点「生成报告」
    for kind in [
        task_kind::INTERVIEW_PREPARE,
        task_kind::INTERVIEW_SYNTHESIZE,
    ] {
        let task_deps = deps.clone();
        registry.on_abandon(kind, move |task, _error| {
            let deps = task_deps.clone();
            async move { prepare::abandon(&deps, &task).await }
        });
    }
    let task_deps = deps.clone();
    registry.on_abandon(task_kind::REPORT_GENERATE, move |task, _error| {
        let deps = task_deps.clone();
        async move { report::abandon(&deps, &task).await }
    });
    registry.on_abandon(task_kind::POSITION_COMPARE, move |task, _error| {
        let deps = deps.clone();
        async move { comparison::abandon(&deps, &task).await }
    });
}
