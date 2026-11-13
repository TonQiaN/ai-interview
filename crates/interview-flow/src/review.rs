//! 面试结束后 HR 看的东西：对话记录、分析报告、结论。

use axum::extract::State;
use axum::http::StatusCode;
use interview_platform::auth::AdminUser;
use interview_platform::http::{ApiError, ApiResult, ErrorBody, Json, Path, Query};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::model::{
    DecisionRequest, InterviewDetail, ReportList, ReportQuery, ReportView, TranscriptView, TurnView,
};
use crate::store::{self, InterviewRow};
use crate::{Deps, finalize, report};

pub(crate) fn router() -> OpenApiRouter<Deps> {
    OpenApiRouter::new()
        .routes(routes!(get_transcript))
        .routes(routes!(get_report))
        .routes(routes!(list_reports))
        .routes(routes!(regenerate_report))
        .routes(routes!(put_decision))
}

async fn load(deps: &Deps, id: Uuid) -> ApiResult<InterviewRow> {
    store::get_interview(&deps.ctx.db, id)
        .await?
        .ok_or_else(|| ApiError::not_found("面试"))
}

/// 对话记录
///
/// 面试官和候选人说的每一段话，按先后排列，带时间戳。时间是相对面试开始的毫秒，和录像同一条时间轴。
/// 面试还没开始时是空的。
#[utoipa::path(
    get,
    path = "/api/v1/interviews/{id}/transcript",
    tag = "interviews",
    params(("id" = Uuid, Path, description = "面试 ID")),
    responses(
        (status = 200, description = "对话记录", body = TranscriptView),
        (status = 404, description = "没有这场面试", body = ErrorBody),
    )
)]
async fn get_transcript(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<TranscriptView>> {
    let row = load(&deps, id).await?;
    let items = store::list_turns(&deps.ctx.db, row.id)
        .await?
        .into_iter()
        .map(|turn| TurnView {
            id: turn.id,
            seq: turn.seq,
            speaker: turn.speaker,
            utterance_key: turn.utterance_key,
            text: turn.text,
            start_ms: turn.start_ms,
            end_ms: turn.end_ms,
            words: turn
                .words
                .and_then(|value| serde_json::from_value(value).ok())
                .unwrap_or_default(),
            confidence: turn.confidence,
            transcript_source: turn.transcript_source,
            end_reason: turn.end_reason,
        })
        .collect();
    Ok(Json(TranscriptView { items }))
}

/// 分析报告
///
/// 最新生成好的那一版；带 `version` 看指定的版本。报告还没生成好（或者生成失败）时是 404，
/// code 是 `report_not_ready`，进度和失败原因看面试详情里的 `report`。
#[utoipa::path(
    get,
    path = "/api/v1/interviews/{id}/report",
    tag = "interviews",
    params(("id" = Uuid, Path, description = "面试 ID"), ReportQuery),
    responses(
        (status = 200, description = "分析报告", body = ReportView),
        (status = 404, description = "没有这场面试，或者报告还没生成好", body = ErrorBody),
    )
)]
async fn get_report(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
    Query(query): Query<ReportQuery>,
) -> ApiResult<Json<ReportView>> {
    let row = load(&deps, id).await?;
    let report = store::report_version(&deps.ctx.db, row.id, query.version)
        .await?
        .ok_or_else(|| ApiError::not_found("分析报告").with_code("report_not_ready"))?;
    Ok(Json(ReportView {
        id: report.id,
        version: report.version,
        generated_at: report.generated_at,
        content: report
            .content
            .ok_or_else(|| ApiError::internal(anyhow::anyhow!("报告标成了已生成，但没有内容")))?,
    }))
}

/// 报告的所有版本
///
/// 新的在前，包括正在生成和生成失败的。
#[utoipa::path(
    get,
    path = "/api/v1/interviews/{id}/reports",
    tag = "interviews",
    params(("id" = Uuid, Path, description = "面试 ID")),
    responses(
        (status = 200, description = "报告的各个版本", body = ReportList),
        (status = 404, description = "没有这场面试", body = ErrorBody),
    )
)]
async fn list_reports(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ReportList>> {
    let row = load(&deps, id).await?;
    let items = store::list_reports(&deps.ctx.db, row.id).await?;
    Ok(Json(ReportList { items }))
}

/// 重新生成报告
///
/// 加一个新版本，在后台生成，旧版本还能看。进度看返回的面试详情里的 `report`。
/// 已经有一份正在生成时不会再加，直接返回当前的情况。面试还没结束时是 409，code 是 `interview_not_finished`。
#[utoipa::path(
    post,
    path = "/api/v1/interviews/{id}/report/regenerate",
    tag = "interviews",
    params(("id" = Uuid, Path, description = "面试 ID")),
    responses(
        (status = 202, description = "开始生成了", body = InterviewDetail),
        (status = 404, description = "没有这场面试", body = ErrorBody),
        (status = 409, description = "面试还没结束", body = ErrorBody),
    )
)]
async fn regenerate_report(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<(StatusCode, Json<InterviewDetail>)> {
    let row = load(&deps, id).await?;
    if !finalize::is_finished(row.status) {
        return Err(
            ApiError::conflict("面试结束之后才能生成报告").with_code("interview_not_finished")
        );
    }
    report::request(&deps, row.id, true).await?;
    let row = load(&deps, id).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(store::detail(&deps.ctx.db, row).await?),
    ))
}

/// 记下 HR 的结论
///
/// 进入下一轮、待定、不通过，可以带一句备注。可以反复改；改回 `pending` 表示还没决定。
#[utoipa::path(
    put,
    path = "/api/v1/interviews/{id}/decision",
    tag = "interviews",
    params(("id" = Uuid, Path, description = "面试 ID")),
    request_body = DecisionRequest,
    responses(
        (status = 200, description = "记下了", body = InterviewDetail),
        (status = 404, description = "没有这场面试", body = ErrorBody),
        (status = 422, description = "备注太长", body = ErrorBody),
    )
)]
async fn put_decision(
    State(deps): State<Deps>,
    user: AdminUser,
    Path(id): Path<Uuid>,
    Json(request): Json<DecisionRequest>,
) -> ApiResult<Json<InterviewDetail>> {
    let row = load(&deps, id).await?;
    let note = request
        .note
        .as_deref()
        .map(str::trim)
        .filter(|note| !note.is_empty());
    if note.is_some_and(|note| note.chars().count() > 2000) {
        return Err(ApiError::invalid_field("note", "备注最多 2000 个字"));
    }
    store::set_decision(&deps.ctx.db, row.id, request.decision, note, user.id).await?;
    let row = load(&deps, id).await?;
    Ok(Json(store::detail(&deps.ctx.db, row).await?))
}
