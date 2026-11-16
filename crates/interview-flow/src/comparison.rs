//! 同一个岗位几位候选人的横向对比：HR 选几场面完的面试，后台拿各自的报告调
//! `interview_report::compare`，结果存在 `position_comparisons`。
//!
//! 一个岗位只留最近一份生成好的对比，和它之后的那次请求（正在生成或者失败了）。
//! 重新对比失败时，上一份生成好的还在，页面照样能看。

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use interview_core::{ReportStatus, task_kind};
use interview_llm::Pricing;
use interview_platform::auth::AdminUser;
use interview_platform::http::{ApiError, ApiResult, ErrorBody, Json, Path};
use interview_platform::tasks::{NewTask, Task, TaskError, TaskQueue, TaskResult};
use interview_platform::usage::{self, LlmUsage};
use interview_report::compare::MAX_CANDIDATES;
use interview_report::record::Job;
use interview_report::report::InterviewReport;
use interview_report::{AnalyzeOptions, CompareError, Comparison, compare};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::prepare::{Stop, failed};
use crate::report::llm_usage;
use crate::{Deps, context, store};

const NO_MODEL: &str = "大模型还没配置（环境变量 DEEPSEEK_API_KEY），做不了对比";

pub(crate) fn router() -> OpenApiRouter<Deps> {
    OpenApiRouter::new().routes(routes!(get_comparison, start_comparison))
}

/// 要对比的几场面试。
#[derive(Debug, Deserialize, ToSchema)]
pub struct CompareRequest {
    /// 2 到 20 场，都要是这个岗位的面试，都要有写好的报告。
    /// 对比里候选人的代号 A、B、C 按这里的顺序给，顺序不影响结果。
    pub interview_ids: Vec<Uuid>,
}

/// 一个岗位的候选人对比。
#[derive(Debug, Serialize, ToSchema)]
pub struct PositionComparison {
    /// 最近一次对比的状态。`pending` 和 `running` 是还在生成，过几秒再取一次。
    pub status: ReportStatus,
    /// 最近一次对比失败的原因。
    pub error: Option<String>,
    pub requested_at: DateTime<Utc>,
    /// 最近一次比的是哪几场面试。
    pub interview_ids: Vec<Uuid>,
    /// 最近一份生成好的对比。最近一次还在生成或者失败了的时候，这里是上一份。
    pub result: Option<ComparisonResult>,
}

/// 一份生成好的对比。
#[derive(Debug, Serialize, ToSchema)]
pub struct ComparisonResult {
    pub generated_at: DateTime<Utc>,
    /// 对比里的每场面试是哪位候选人。对比的正文里只有代号和面试 ID，姓名从这里拿。
    pub people: Vec<ComparedPerson>,
    pub comparison: Comparison,
}

/// 对比里的一场面试对应的候选人。
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct ComparedPerson {
    pub interview_id: Uuid,
    pub candidate_id: Uuid,
    pub name: String,
}

/// `position.compare` 的 payload。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparePayload {
    pub comparison_id: Uuid,
}

#[derive(Debug, sqlx::FromRow)]
struct Row {
    id: Uuid,
    position_id: Uuid,
    interview_ids: Vec<Uuid>,
    status: ReportStatus,
    content: Option<Value>,
    error: Option<String>,
    generated_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

const COLUMNS: &str =
    "id, position_id, interview_ids, status, content, error, generated_at, created_at";

/// 候选人对比
///
/// 这个岗位最近一次对比的状态，和最近一份生成好的对比。从来没比过是 404。
#[utoipa::path(
    get,
    path = "/api/v1/positions/{id}/comparison",
    tag = "interviews",
    params(("id" = Uuid, Path, description = "岗位 ID")),
    responses(
        (status = 200, description = "对比", body = PositionComparison),
        (status = 404, description = "没有这个岗位，或者还没有比过", body = ErrorBody),
    )
)]
async fn get_comparison(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(position_id): Path<Uuid>,
) -> ApiResult<Json<PositionComparison>> {
    let db = &deps.ctx.db;
    let latest = latest(db, position_id)
        .await?
        .ok_or_else(|| ApiError::not_found("候选人对比").with_code("comparison_not_found"))?;
    Ok(Json(view(db, latest).await?))
}

/// 对比几位候选人
///
/// 拿这几场面试各自最新的报告做横向对比：先约谁、各自强在哪弱在哪、岗位要求对照表、
/// 分不出高下时人工面试问什么。在后台生成（5 份报告约一分钟），进度用 GET 看。
/// 同样的几场正在比的时候不会再排一次，直接返回当前的情况；正在比别的几场时是 409。
#[utoipa::path(
    post,
    path = "/api/v1/positions/{id}/comparison",
    tag = "interviews",
    params(("id" = Uuid, Path, description = "岗位 ID")),
    request_body = CompareRequest,
    responses(
        (status = 202, description = "开始生成了", body = PositionComparison),
        (status = 404, description = "没有这个岗位", body = ErrorBody),
        (status = 409, description = "上一次对比还在生成，或者大模型没配置", body = ErrorBody),
        (status = 422, description = "选的面试不对：少于 2 场、多于 20 场、不是这个岗位的、还没有报告", body = ErrorBody),
    )
)]
async fn start_comparison(
    State(deps): State<Deps>,
    user: AdminUser,
    Path(position_id): Path<Uuid>,
    Json(request): Json<CompareRequest>,
) -> ApiResult<(StatusCode, Json<PositionComparison>)> {
    let db = &deps.ctx.db;
    let ids = request.interview_ids;
    let bad = |message: String| ApiError::invalid_field("interview_ids", message);
    if ids.len() < 2 {
        return Err(bad("至少选两场面试才能比较".to_owned()));
    }
    if ids.len() > MAX_CANDIDATES {
        return Err(bad(format!("一次最多比较 {MAX_CANDIDATES} 场面试")));
    }
    if ids.iter().collect::<HashSet<_>>().len() != ids.len() {
        return Err(bad("同一场面试选了两次".to_owned()));
    }

    let mut tx = db.begin().await?;
    // 锁住岗位这一行：同一时间只有一个事务能给它加新的对比
    let found = sqlx::query("SELECT 1 FROM positions WHERE id = $1 FOR UPDATE")
        .bind(position_id)
        .execute(&mut *tx)
        .await?;
    if found.rows_affected() == 0 {
        return Err(ApiError::not_found("岗位"));
    }
    let known: Vec<(Uuid, bool)> = sqlx::query_as(
        "SELECT i.id, EXISTS (SELECT 1 FROM interview_reports r \
                               WHERE r.interview_id = i.id AND r.status = 'done') \
         FROM interviews i WHERE i.id = ANY($1) AND i.position_id = $2",
    )
    .bind(&ids)
    .bind(position_id)
    .fetch_all(&mut *tx)
    .await?;
    if known.len() != ids.len() {
        return Err(bad(format!(
            "有 {} 场面试不是这个岗位的，或者已经不存在",
            ids.len() - known.len()
        )));
    }
    let without_report = known.iter().filter(|(_, has_report)| !has_report).count();
    if without_report > 0 {
        return Err(bad(format!(
            "有 {without_report} 场面试还没有写好的报告，等报告写好再比"
        )));
    }
    if deps.llm.is_none() {
        return Err(ApiError::conflict(NO_MODEL));
    }

    let row = match latest_in(&mut tx, position_id).await? {
        Some(current) if in_progress(current.status) => {
            let same = current.interview_ids.iter().collect::<HashSet<_>>()
                == ids.iter().collect::<HashSet<_>>();
            if !same {
                return Err(ApiError::conflict("上一次对比还在生成，等它完成再比")
                    .with_code("comparison_in_progress"));
            }
            current
        }
        _ => {
            let row: Row = sqlx::query_as(&format!(
                "INSERT INTO position_comparisons (id, position_id, interview_ids, requested_by) \
                 VALUES ($1, $2, $3, $4) RETURNING {COLUMNS}"
            ))
            .bind(interview_core::new_id())
            .bind(position_id)
            .bind(&ids)
            .bind(user.id)
            .fetch_one(&mut *tx)
            .await?;
            let payload = ComparePayload {
                comparison_id: row.id,
            };
            TaskQueue::enqueue_in(
                &mut tx,
                NewTask::new(task_kind::POSITION_COMPARE, json!(payload))
                    .dedupe(format!("position.compare:{}", row.id)),
            )
            .await?;
            row
        }
    };
    tx.commit().await?;
    Ok((StatusCode::ACCEPTED, Json(view(db, row).await?)))
}

fn in_progress(status: ReportStatus) -> bool {
    matches!(status, ReportStatus::Pending | ReportStatus::Running)
}

async fn latest(db: &PgPool, position_id: Uuid) -> Result<Option<Row>, sqlx::Error> {
    let mut conn = db.acquire().await?;
    latest_in(&mut conn, position_id).await
}

async fn latest_in(conn: &mut PgConnection, position_id: Uuid) -> Result<Option<Row>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM position_comparisons \
         WHERE position_id = $1 ORDER BY created_at DESC, id DESC LIMIT 1"
    ))
    .bind(position_id)
    .fetch_optional(conn)
    .await
}

/// 给页面看的样子：最近一次的状态，加上最近一份生成好的对比。
async fn view(db: &PgPool, latest: Row) -> ApiResult<PositionComparison> {
    let done = if latest.status == ReportStatus::Done {
        None
    } else {
        sqlx::query_as::<_, Row>(&format!(
            "SELECT {COLUMNS} FROM position_comparisons \
             WHERE position_id = $1 AND status = 'done' \
             ORDER BY created_at DESC, id DESC LIMIT 1"
        ))
        .bind(latest.position_id)
        .fetch_optional(db)
        .await?
    };
    let result = match done.as_ref().unwrap_or(&latest) {
        row if row.status == ReportStatus::Done => result(db, row).await?,
        _ => None,
    };
    Ok(PositionComparison {
        status: latest.status,
        error: latest.error,
        requested_at: latest.created_at,
        interview_ids: latest.interview_ids,
        result,
    })
}

async fn result(db: &PgPool, row: &Row) -> Result<Option<ComparisonResult>, sqlx::Error> {
    let (Some(content), Some(generated_at)) = (row.content.clone(), row.generated_at) else {
        return Ok(None);
    };
    // 结构对不上（比如以后改了版本）就当没有，让 HR 重新比一次
    let Ok(comparison) = serde_json::from_value::<Comparison>(content) else {
        tracing::warn!(comparison_id = %row.id, "存下来的对比读不出来，结构可能变过");
        return Ok(None);
    };
    let found: Vec<ComparedPerson> = sqlx::query_as(
        "SELECT i.id AS interview_id, c.id AS candidate_id, c.name \
         FROM interviews i JOIN candidates c ON c.id = i.candidate_id \
         WHERE i.id = ANY($1)",
    )
    .bind(&row.interview_ids)
    .fetch_all(db)
    .await?;
    // 按比的时候的顺序排，和代号 A、B、C 对得上
    let mut by_id: HashMap<Uuid, ComparedPerson> = found
        .into_iter()
        .map(|person| (person.interview_id, person))
        .collect();
    let people = row
        .interview_ids
        .iter()
        .filter_map(|id| by_id.remove(id))
        .collect();
    Ok(Some(ComparisonResult {
        generated_at,
        people,
        comparison,
    }))
}

/// 任务入口。
pub async fn run(deps: &Deps, task: &Task) -> TaskResult {
    let payload: ComparePayload = task.payload_as()?;
    let db = &deps.ctx.db;
    let id = payload.comparison_id;
    let started = sqlx::query(
        "UPDATE position_comparisons SET status = 'running', error = NULL \
         WHERE id = $1 AND status IN ('pending', 'running')",
    )
    .bind(id)
    .execute(db)
    .await?;
    if started.rows_affected() == 0 {
        return Ok(Some(json!({ "skipped": "这份对比已经不需要生成了" })));
    }
    match generate(deps, task, id).await {
        Ok(summary) => Ok(Some(summary)),
        Err(Stop::Failed(message)) => {
            tracing::warn!(comparison_id = %id, reason = %message, "候选人对比生成失败");
            fail(db, id, &message).await?;
            Ok(Some(json!({ "failed": message })))
        }
        Err(Stop::Retry(err)) if task.attempts >= task.max_attempts => {
            let message = format!("生成对比出错，重试了 {} 次还是不行：{err:#}", task.attempts);
            fail(db, id, &message).await?;
            Err(TaskError::Fatal(err))
        }
        Err(Stop::Retry(err)) => Err(TaskError::Retry(err)),
    }
}

async fn generate(deps: &Deps, task: &Task, id: Uuid) -> Result<Value, Stop> {
    let db = &deps.ctx.db;
    let Some(model) = deps.llm.as_deref() else {
        return Err(failed(NO_MODEL));
    };
    let row: Option<Row> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM position_comparisons WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(db)
    .await?;
    let Some(row) = row else {
        return Err(failed("这份对比已经不存在"));
    };

    let mut reports = Vec::with_capacity(row.interview_ids.len());
    // 岗位的硬性要求和加分项是出题时整理的，每场面试各存了一份，用第一场有题目的
    let mut brief = None;
    let mut thinking = None;
    for interview_id in &row.interview_ids {
        let Some(interview) = store::get_interview(db, *interview_id).await? else {
            return Err(failed("选的面试里有一场已经被删除，重新选人再比"));
        };
        let content = store::report_version(db, interview.id, None)
            .await?
            .and_then(|report| report.content);
        let Some(content) = content else {
            return Err(failed("选的面试里有一场没有写好的报告，重新生成报告后再比"));
        };
        let Ok(report) = serde_json::from_value::<InterviewReport>(content) else {
            return Err(failed(
                "选的面试里有一份报告是旧版本的，读不出来。重新生成那份报告后再比",
            ));
        };
        reports.push(report);
        if brief.is_none()
            && let Ok(Some(plan)) = interview.plan()
        {
            brief = Some(plan.brief);
        }
        if thinking.is_none()
            && let Ok(config) = interview.config()
        {
            // 和写报告一样，用出题那一档思考
            thinking = Some(config.planning_thinking);
        }
    }

    let job = context::job(db, row.position_id).await?;
    let job = Job {
        title: job.title,
        description: job.description,
        must_have: brief
            .as_ref()
            .map(|b| b.must_haves.clone())
            .unwrap_or_default(),
        nice_to_have: brief
            .as_ref()
            .map(|b| b.nice_to_haves.clone())
            .unwrap_or_default(),
    };
    let options = AnalyzeOptions {
        thinking,
        ..AnalyzeOptions::default()
    };
    let started = Instant::now();
    let refs: Vec<&InterviewReport> = reports.iter().collect();
    let comparison = compare(&job, &refs, model, &options)
        .await
        .map_err(|err| match err {
            // 多半是模型服务一时不稳，过一会儿再试
            CompareError::Model(message) => Stop::Retry(anyhow::anyhow!(message)),
            other => failed(other.to_string()),
        })?;

    let meta = &comparison.meta;
    let spent = llm_usage(&meta.usage);
    usage::record(
        db,
        LlmUsage {
            purpose: "position.compare",
            model: &meta.model,
            thinking: Some(meta.thinking.as_str()),
            prompt_tokens: spent.prompt_tokens,
            completion_tokens: spent.completion_tokens,
            reasoning_tokens: spent.reasoning_tokens,
            cached_tokens: spent.cache_hit_tokens,
            cost_cny: Some(Pricing::FLASH_PEAK.cost_cny(&spent)),
            latency_ms: Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
            interview_id: None,
            task_id: Some(task.id),
        },
    )
    .await;

    let summary = json!({ "candidates": reports.len(), "calls": meta.usage.calls });
    let mut tx = db.begin().await?;
    sqlx::query(
        "UPDATE position_comparisons SET status = 'done', content = $2, error = NULL, \
             generated_at = now() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(serde_json::to_value(&comparison)?)
    .execute(&mut *tx)
    .await?;
    // 新的一份好了，之前的都不留
    sqlx::query("DELETE FROM position_comparisons WHERE position_id = $1 AND id <> $2")
        .bind(row.position_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(summary)
}

/// 标成失败。上一份生成好的留着，更早的失败记录不留。
async fn fail(db: &PgPool, id: Uuid, error: &str) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;
    let position: Option<Uuid> = sqlx::query_scalar(
        "UPDATE position_comparisons SET status = 'failed', error = $2 \
         WHERE id = $1 AND status IN ('pending', 'running') RETURNING position_id",
    )
    .bind(id)
    .bind(error)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(position_id) = position {
        sqlx::query(
            "DELETE FROM position_comparisons \
             WHERE position_id = $1 AND id <> $2 AND status <> 'done'",
        )
        .bind(position_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}
