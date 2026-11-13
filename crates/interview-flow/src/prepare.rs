//! 面试前的准备：出题，把面试官要说的话写进库，预先合成语音。
//!
//! 在后台任务里跑（`interview.prepare` 和 `interview.synthesize` 共用这一个入口）。
//! 已经有题目就不重新出，只补上没合成好的语音，所以这个任务重复执行是安全的。
//!
//! 结果写在面试上：成功 `ready`，失败 `prepare_failed` 加 `prepare_error`。
//! HR 能自己处理的失败（没配大模型、音色不能用、某几句合成失败）不重试，直接告诉 HR；
//! 数据库抖动、简历还在解析这类过一会儿就好的才交给任务队列重试。

use std::time::Instant;

use bytes::Bytes;
use interview_core::{InterviewStatus, SynthStatus, blob_key};
use interview_engine::{
    CandidateContext, EngineError, InterviewConfig, InterviewPlan, JobContext, PlanInput,
    plan_interview,
};
use interview_llm::Pricing;
use interview_platform::tasks::{NewTask, Task, TaskError, TaskQueue, TaskResult};
use interview_platform::usage::{self, LlmUsage};
use interview_tts::BatchItem;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::store::{self, InterviewRow, SynthResult};
use crate::{Deps, context, voice};

/// 两个任务共用的 payload。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparePayload {
    pub interview_id: Uuid,
}

/// 入队一个准备任务。和改状态放在同一个事务里，保证「状态是 preparing」和「有任务在跑」同时成立。
///
/// 不用 `dedupe`：调用方先用条件更新把状态改成 `preparing`，同一时间只有一个请求能改成功，
/// 重复入队已经挡住了。加了反而会出错：上一条任务把面试改成 `ready` 之后、任务队列把它记成
/// 做完之前，新的请求会搭在那条马上结束的任务上，面试就一直停在 `preparing`。
pub(crate) async fn enqueue(
    conn: &mut PgConnection,
    kind: &'static str,
    interview_id: Uuid,
) -> Result<Uuid, sqlx::Error> {
    TaskQueue::enqueue_in(
        conn,
        NewTask::new(kind, json!(PreparePayload { interview_id })),
    )
    .await
}

/// 后台任务没做成。
pub(crate) enum Stop {
    /// 要 HR 处理，原因写在面试上
    Failed(String),
    /// 过一会儿再试
    Retry(anyhow::Error),
}

impl<E: Into<anyhow::Error>> From<E> for Stop {
    fn from(err: E) -> Self {
        Stop::Retry(err.into())
    }
}

pub(crate) fn failed(message: impl Into<String>) -> Stop {
    Stop::Failed(message.into())
}

/// 任务入口。
pub async fn run(deps: &Deps, task: &Task) -> TaskResult {
    let payload: PreparePayload = task.payload_as()?;
    let id = payload.interview_id;
    let Some(row) = store::get_interview(&deps.ctx.db, id).await? else {
        return Ok(Some(json!({ "skipped": "面试已经不存在" })));
    };
    if row.status != InterviewStatus::Preparing {
        // HR 中途取消了，或者是一条过期的任务
        return Ok(Some(
            json!({ "skipped": format!("面试的状态是 {}", row.status) }),
        ));
    }

    let outcome = prepare(deps, task, &row).await;
    let mut conn = deps.ctx.db.acquire().await?;
    let from = [InterviewStatus::Preparing];
    match outcome {
        Ok(summary) => {
            store::set_status(&mut conn, id, &from, InterviewStatus::Ready, None).await?;
            Ok(Some(summary))
        }
        Err(Stop::Failed(message)) => {
            tracing::warn!(interview_id = %id, reason = %message, "面试准备失败");
            store::set_status(
                &mut conn,
                id,
                &from,
                InterviewStatus::PrepareFailed,
                Some(&message),
            )
            .await?;
            Ok(Some(json!({ "failed": message })))
        }
        Err(Stop::Retry(err)) if task.attempts >= task.max_attempts => {
            let message = format!("准备过程出错，重试了 {} 次还是不行：{err:#}", task.attempts);
            store::set_status(
                &mut conn,
                id,
                &from,
                InterviewStatus::PrepareFailed,
                Some(&message),
            )
            .await?;
            Err(TaskError::Fatal(err))
        }
        Err(Stop::Retry(err)) => Err(TaskError::Retry(err)),
    }
}

async fn prepare(deps: &Deps, task: &Task, row: &InterviewRow) -> Result<serde_json::Value, Stop> {
    let config = row
        .config()
        .map_err(|e| failed(format!("面试配置读不出来：{e}")))?;
    let plan = match row
        .plan()
        .map_err(|e| failed(format!("已有的题目读不出来，请重新出题：{e}")))?
    {
        Some(plan) => plan,
        None => {
            let plan = generate_plan(deps, task, row, &config).await?;
            let mut tx = deps.ctx.db.begin().await?;
            sqlx::query("UPDATE interviews SET plan = $2 WHERE id = $1")
                .bind(row.id)
                .bind(serde_json::to_value(&plan)?)
                .execute(&mut *tx)
                .await?;
            store::sync_utterances(&mut tx, row.id, &plan.utterances(), false).await?;
            tx.commit().await?;
            plan
        }
    };

    let synthesized = synthesize(deps, row, &config).await?;
    Ok(json!({
        "questions": plan.questions.len(),
        "utterances": plan.utterances().len(),
        "synthesized": synthesized,
    }))
}

/// 调大模型出题。
async fn generate_plan(
    deps: &Deps,
    task: &Task,
    row: &InterviewRow,
    config: &InterviewConfig,
) -> Result<InterviewPlan, Stop> {
    let Some(model) = deps.llm.as_deref() else {
        return Err(failed(
            "大模型还没配置（环境变量 DEEPSEEK_API_KEY），出不了题",
        ));
    };
    let db = &deps.ctx.db;

    let job = context::job(db, row.position_id).await?;
    let (name,): (String,) = sqlx::query_as("SELECT name FROM candidates WHERE id = $1")
        .bind(row.candidate_id)
        .fetch_one(db)
        .await?;
    let resume = context::resume(db, row.candidate_id, row.resume_id).await?;
    if resume.parsing {
        return Err(Stop::Retry(anyhow::anyhow!(
            "简历还在解析，等它解析完再出题"
        )));
    }
    let input = PlanInput {
        job: JobContext {
            title: job.title,
            company: job.company,
            description: job.description,
        },
        candidate: CandidateContext {
            name,
            resume_text: resume.text,
        },
        config: config.clone(),
    };

    let started = Instant::now();
    let plan = plan_interview(model, &input)
        .await
        .map_err(|err| match err {
            EngineError::Llm(llm) if llm.is_retryable() => Stop::Retry(llm.into()),
            other => failed(format!("出题失败：{other}")),
        })?;

    let generation = &plan.generation;
    let thinking = serde_json::to_value(config.planning_thinking)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned));
    usage::record(
        db,
        LlmUsage {
            purpose: "interview.plan",
            model: &generation.model,
            thinking: thinking.as_deref(),
            prompt_tokens: generation.usage.prompt_tokens,
            completion_tokens: generation.usage.completion_tokens,
            reasoning_tokens: generation.usage.reasoning_tokens,
            cached_tokens: generation.usage.cache_hit_tokens,
            cost_cny: Some(Pricing::FLASH_PEAK.cost_cny(&generation.usage)),
            latency_ms: Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
            interview_id: Some(row.id),
            task_id: Some(task.id),
        },
    )
    .await;
    Ok(plan)
}

/// 合成还没有音频的话。返回这次合成了几句。
async fn synthesize(
    deps: &Deps,
    row: &InterviewRow,
    config: &InterviewConfig,
) -> Result<usize, Stop> {
    let db = &deps.ctx.db;
    let pending = store::pending_utterances(db, row.id).await?;
    if pending.is_empty() {
        return Ok(0);
    }
    let voice = voice::resolve(&deps.ctx, row.voice_id, config.language)
        .await
        .map_err(failed)?;
    let engine = voice.engine.as_str();

    let items = pending
        .iter()
        .map(|(key, text)| BatchItem {
            key: key.clone(),
            request: voice.request(text),
        })
        .collect();
    let outcomes = deps.synth.synthesize_batch(items).await;

    let mut failures: Vec<String> = Vec::new();
    for outcome in &outcomes {
        let saved = match &outcome.result {
            Ok(audio) => {
                let key = blob_key::utterance_audio(row.id, &outcome.key);
                deps.ctx
                    .blobs
                    .put(&key, Bytes::from(audio.wav.clone()))
                    .await?;
                store::save_synth_result(
                    db,
                    row.id,
                    SynthResult {
                        key: &outcome.key,
                        status: SynthStatus::Done,
                        engine,
                        blob_key: Some(&key),
                        duration_ms: Some(i32::try_from(audio.duration_ms()).unwrap_or(i32::MAX)),
                        error: None,
                    },
                )
                .await
            }
            Err(err) => {
                let message = err.to_string();
                failures.push(format!("{}（{message}）", outcome.key));
                store::save_synth_result(
                    db,
                    row.id,
                    SynthResult {
                        key: &outcome.key,
                        status: SynthStatus::Failed,
                        engine,
                        blob_key: None,
                        duration_ms: None,
                        error: Some(&message),
                    },
                )
                .await
            }
        };
        saved?;
    }

    if failures.is_empty() {
        Ok(outcomes.len())
    } else {
        let shown: Vec<&str> = failures.iter().take(3).map(String::as_str).collect();
        Err(failed(format!(
            "{} 句语音合成失败：{}{}。题目已经保留，处理好后再点一次准备，只会补合成失败的这几句",
            failures.len(),
            shown.join("、"),
            if failures.len() > shown.len() {
                " 等"
            } else {
                ""
            },
        )))
    }
}
