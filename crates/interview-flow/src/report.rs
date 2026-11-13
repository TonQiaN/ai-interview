//! 分析报告：决定要不要生成、在后台任务里调 `interview-report` 生成、把结果存下来。
//!
//! 一场面试可以有多个版本的报告（`interview_reports.version`），HR 每点一次「重新生成」加一版。
//! 进度和失败原因写在报告这一行上（`status`、`error`），HR 在面试详情里能看到。

use std::time::Instant;

use interview_core::{ReportStatus, task_kind};
use interview_llm::{Pricing, Usage};
use interview_platform::tasks::{NewTask, Task, TaskError, TaskQueue, TaskResult};
use interview_platform::usage::{self, LlmUsage};
use interview_report::{AnalyzeError, AnalyzeOptions, SCHEMA_VERSION, analyze};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::prepare::{Stop, failed};
use crate::store::{self, ReportRow};
use crate::{Deps, record};

const NO_ANSWERS: &str = "候选人没有作答，没有可以分析的内容";
const NO_MODEL: &str =
    "大模型还没配置（环境变量 DEEPSEEK_API_KEY），写不了报告。配置好之后点「重新生成」";

/// `report.generate` 的 payload。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportPayload {
    pub interview_id: Uuid,
    pub report_id: Uuid,
}

/// 要一份报告。
///
/// - 有一份正在生成：直接返回它，不加新版本。
/// - `regenerate` 为假（面试结束后自动出报告）：已经有报告就返回最新的那份。
/// - 其余情况加一个新版本，入队生成任务。现在就知道生成不了的（候选人没作答、没配大模型），
///   新版本直接是 `failed`，原因写在上面。
pub(crate) async fn request(
    deps: &Deps,
    interview_id: Uuid,
    regenerate: bool,
) -> Result<ReportRow, sqlx::Error> {
    let db = &deps.ctx.db;
    let turns = store::list_turns(db, interview_id).await?;
    let blocker = if !record::has_answers(&turns) {
        Some(NO_ANSWERS)
    } else if deps.llm.is_none() {
        Some(NO_MODEL)
    } else {
        None
    };

    let mut tx = db.begin().await?;
    if let Some(latest) = store::lock_latest_report(&mut tx, interview_id).await? {
        let in_progress = matches!(latest.status, ReportStatus::Pending | ReportStatus::Running);
        if in_progress || !regenerate {
            return Ok(latest);
        }
    }
    let report = match blocker {
        Some(reason) => {
            store::insert_report(&mut tx, interview_id, ReportStatus::Failed, Some(reason)).await?
        }
        None => {
            let report =
                store::insert_report(&mut tx, interview_id, ReportStatus::Pending, None).await?;
            let payload = ReportPayload {
                interview_id,
                report_id: report.id,
            };
            TaskQueue::enqueue_in(
                &mut tx,
                NewTask::new(task_kind::REPORT_GENERATE, json!(payload))
                    .dedupe(format!("report.generate:{}", report.id)),
            )
            .await?;
            report
        }
    };
    tx.commit().await?;
    Ok(report)
}

/// 任务入口。
pub async fn run(deps: &Deps, task: &Task) -> TaskResult {
    let payload: ReportPayload = task.payload_as()?;
    let db = &deps.ctx.db;
    if !store::start_report(db, payload.report_id).await? {
        return Ok(Some(json!({ "skipped": "这份报告已经不需要生成了" })));
    }
    match generate(deps, task, &payload).await {
        Ok(summary) => Ok(Some(summary)),
        Err(Stop::Failed(message)) => {
            tracing::warn!(interview_id = %payload.interview_id, reason = %message, "分析报告生成失败");
            store::fail_report(db, payload.report_id, &message).await?;
            Ok(Some(json!({ "failed": message })))
        }
        Err(Stop::Retry(err)) if task.attempts >= task.max_attempts => {
            let message = format!("生成报告出错，重试了 {} 次还是不行：{err:#}", task.attempts);
            store::fail_report(db, payload.report_id, &message).await?;
            Err(TaskError::Fatal(err))
        }
        Err(Stop::Retry(err)) => Err(TaskError::Retry(err)),
    }
}

async fn generate(deps: &Deps, task: &Task, payload: &ReportPayload) -> Result<Value, Stop> {
    let db = &deps.ctx.db;
    let Some(model) = deps.llm.as_deref() else {
        return Err(failed(NO_MODEL));
    };
    let Some(row) = store::get_interview(db, payload.interview_id).await? else {
        return Err(failed("面试已经不存在"));
    };
    let turns = store::list_turns(db, row.id).await?;
    if !record::has_answers(&turns) {
        return Err(failed(NO_ANSWERS));
    }
    let config = row
        .config()
        .map_err(|e| failed(format!("面试配置读不出来：{e}")))?;
    let record = record::assemble(db, &row, &turns).await?;

    // 写报告不赶时间，和出题用同一档思考
    let options = AnalyzeOptions {
        thinking: Some(config.planning_thinking),
        ..AnalyzeOptions::default()
    };
    let started = Instant::now();
    let report = analyze(&record, model, &options)
        .await
        .map_err(|err| match err {
            AnalyzeError::NothingToAnalyze => failed(NO_ANSWERS),
            // 多半是模型服务一时不稳，过一会儿再试
            other => Stop::Retry(anyhow::anyhow!(other)),
        })?;

    let meta = &report.meta;
    let spent = Usage {
        prompt_tokens: meta.usage.prompt_tokens,
        completion_tokens: meta.usage.completion_tokens,
        reasoning_tokens: meta.usage.reasoning_tokens,
        cache_hit_tokens: meta.usage.cache_hit_tokens,
        cache_miss_tokens: meta.usage.cache_miss_tokens,
    };
    usage::record(
        db,
        LlmUsage {
            purpose: "report.generate",
            model: &meta.model,
            thinking: Some(meta.thinking.as_str()),
            prompt_tokens: spent.prompt_tokens,
            completion_tokens: spent.completion_tokens,
            reasoning_tokens: spent.reasoning_tokens,
            cached_tokens: spent.cache_hit_tokens,
            cost_cny: Some(Pricing::FLASH_PEAK.cost_cny(&spent)),
            latency_ms: Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
            interview_id: Some(row.id),
            task_id: Some(task.id),
        },
    )
    .await;

    let summary = json!({
        "calls": meta.usage.calls,
        "failed_stages": meta.failed_stages.len(),
    });
    let schema_version = i32::try_from(SCHEMA_VERSION).unwrap_or(i32::MAX);
    store::finish_report(
        db,
        payload.report_id,
        serde_json::to_value(&report)?,
        schema_version,
    )
    .await?;
    Ok(summary)
}
