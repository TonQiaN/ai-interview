//! 面试场次的接口。

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use bytes::Bytes;
use interview_core::{InterviewStatus, task_kind};
use interview_engine::{ConfigLimits, InterviewConfig, InterviewPlan};
use interview_platform::auth::AdminUser;
use interview_platform::http::{
    ApiError, ApiResult, ErrorBody, Json, Page, PageQuery, Path, Query, serve_blob,
};
use serde_json::{Map, Value};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::model::{
    ConfigDefaults, CreateInterview, InterviewDetail, InterviewFilter, InterviewSummary,
    PrepareRequest, UpdateInterview, UtteranceView,
};
use crate::store::{self, InterviewRow, NewInterview};
use crate::{Deps, prepare};

pub(crate) fn router() -> OpenApiRouter<Deps> {
    OpenApiRouter::new()
        .routes(routes!(list_interviews, create_interview))
        .routes(routes!(get_interview, update_interview, archive_interview))
        .routes(routes!(prepare_interview))
        .routes(routes!(cancel_interview))
        .routes(routes!(get_plan, put_plan))
        .routes(routes!(list_utterances))
        .routes(routes!(utterance_audio))
        .routes(routes!(config_defaults))
}

/// HR 可以改配置、改题、重新准备的状态。
const EDITABLE: [InterviewStatus; 3] = [
    InterviewStatus::Draft,
    InterviewStatus::PrepareFailed,
    InterviewStatus::Ready,
];

async fn load(deps: &Deps, id: Uuid) -> ApiResult<InterviewRow> {
    store::get_interview(&deps.ctx.db, id)
        .await?
        .ok_or_else(|| ApiError::not_found("面试"))
}

async fn detail(deps: &Deps, id: Uuid) -> ApiResult<InterviewDetail> {
    let row = load(deps, id).await?;
    Ok(store::detail(&deps.ctx.db, row).await?)
}

/// 现在这个状态为什么不能改。
fn ensure_editable(status: InterviewStatus) -> ApiResult<()> {
    if EDITABLE.contains(&status) {
        return Ok(());
    }
    let reason = match status {
        InterviewStatus::Preparing => "正在准备，等它结束或者先取消",
        InterviewStatus::Invited => "已经发了面试链接，先作废链接再改",
        _ => "面试已经开始或者结束，不能再改",
    };
    Err(ApiError::conflict(reason).with_code("interview_not_editable"))
}

pub(crate) fn check_config(config: &InterviewConfig) -> ApiResult<()> {
    let issues = config.validate();
    if issues.is_empty() {
        return Ok(());
    }
    let details: Map<String, Value> = issues
        .iter()
        .map(|issue| (issue.field.clone(), Value::String(issue.message.clone())))
        .collect();
    Err(ApiError::validation(
        format!("面试配置有 {} 处问题", issues.len()),
        Value::Object(details),
    ))
}

async fn check_resume(deps: &Deps, resume_id: Uuid, candidate_id: Uuid) -> ApiResult<()> {
    if store::resume_belongs_to(&deps.ctx.db, resume_id, candidate_id).await? {
        Ok(())
    } else {
        Err(ApiError::invalid_field(
            "resume_id",
            "这份简历不存在，或者不是这个候选人的",
        ))
    }
}

async fn check_voice(deps: &Deps, voice_id: Uuid) -> ApiResult<()> {
    if store::exists(&deps.ctx.db, "voices", voice_id).await? {
        Ok(())
    } else {
        Err(ApiError::invalid_field("voice_id", "这个音色不存在"))
    }
}

/// 面试列表
///
/// 可以按候选人、岗位、状态筛选；`q` 搜候选人姓名和岗位名称。默认按创建时间倒序。
#[utoipa::path(
    get,
    path = "/api/v1/interviews",
    tag = "interviews",
    params(InterviewFilter, PageQuery),
    responses((status = 200, description = "一页面试", body = Page<InterviewSummary>))
)]
async fn list_interviews(
    State(deps): State<Deps>,
    _user: AdminUser,
    Query(filter): Query<InterviewFilter>,
    Query(query): Query<PageQuery>,
) -> ApiResult<Json<Page<InterviewSummary>>> {
    let (items, total) = store::list_interviews(&deps.ctx.db, &filter, &query).await?;
    Ok(Json(Page::new(items, total, &query)))
}

/// 新建面试
///
/// 建好是草稿（`draft`），还没出题。配置的优先顺序：请求里的 `config`、模板的配置、默认配置。
#[utoipa::path(
    post,
    path = "/api/v1/interviews",
    tag = "interviews",
    request_body = CreateInterview,
    responses(
        (status = 201, description = "建好了", body = InterviewDetail),
        (status = 422, description = "候选人、岗位不存在，或者配置不合要求", body = ErrorBody),
    )
)]
async fn create_interview(
    State(deps): State<Deps>,
    user: AdminUser,
    Json(request): Json<CreateInterview>,
) -> ApiResult<(StatusCode, Json<InterviewDetail>)> {
    let db = &deps.ctx.db;
    if !store::exists(db, "candidates", request.candidate_id).await? {
        return Err(ApiError::invalid_field("candidate_id", "这个候选人不存在"));
    }
    if !store::exists(db, "positions", request.position_id).await? {
        return Err(ApiError::invalid_field("position_id", "这个岗位不存在"));
    }
    if let Some(resume_id) = request.resume_id {
        check_resume(&deps, resume_id, request.candidate_id).await?;
    }
    if let Some(voice_id) = request.voice_id {
        check_voice(&deps, voice_id).await?;
    }
    let template_config = match request.template_id {
        Some(template_id) => Some(
            crate::templates::template_config(db, template_id)
                .await?
                .ok_or_else(|| ApiError::invalid_field("template_id", "这个模板不存在"))?,
        ),
        None => None,
    };
    let config = request.config.or(template_config).unwrap_or_default();
    check_config(&config)?;

    let row = store::insert_interview(
        db,
        NewInterview {
            candidate_id: request.candidate_id,
            position_id: request.position_id,
            resume_id: request.resume_id,
            template_id: request.template_id,
            voice_id: request.voice_id,
            config: &config,
            created_by: user.id,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(store::detail(db, row).await?)))
}

/// 面试详情
///
/// 带候选人和岗位的摘要、语音合成进度、最新一份报告的状态。
#[utoipa::path(
    get,
    path = "/api/v1/interviews/{id}",
    tag = "interviews",
    params(("id" = Uuid, Path, description = "面试 ID")),
    responses(
        (status = 200, description = "详情", body = InterviewDetail),
        (status = 404, description = "没有这场面试", body = ErrorBody),
    )
)]
async fn get_interview(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<InterviewDetail>> {
    Ok(Json(detail(&deps, id).await?))
}

/// 改面试
///
/// 只能在发链接之前改。改了配置或简历，已经出好的题目会作废、面试回到草稿，要重新准备；
/// 只换音色的话题目保留，重新准备时只重新合成语音。
#[utoipa::path(
    patch,
    path = "/api/v1/interviews/{id}",
    tag = "interviews",
    params(("id" = Uuid, Path, description = "面试 ID")),
    request_body = UpdateInterview,
    responses(
        (status = 200, description = "改好了", body = InterviewDetail),
        (status = 409, description = "现在的状态不能改，code 是 interview_not_editable", body = ErrorBody),
        (status = 422, description = "配置不合要求", body = ErrorBody),
    )
)]
async fn update_interview(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateInterview>,
) -> ApiResult<Json<InterviewDetail>> {
    let row = load(&deps, id).await?;
    ensure_editable(row.status)?;

    let old_config = row.config().map_err(ApiError::internal)?;
    let config = request.config.unwrap_or_else(|| old_config.clone());
    check_config(&config)?;
    let resume_id = request.resume_id.or(row.resume_id);
    if let Some(resume_id) = request.resume_id {
        check_resume(&deps, resume_id, row.candidate_id).await?;
    }
    let voice_id = request.voice_id.or(row.voice_id);
    if let Some(voice_id) = request.voice_id {
        check_voice(&deps, voice_id).await?;
    }

    let content_changed = config != old_config || resume_id != row.resume_id;
    let voice_changed = voice_id != row.voice_id;
    // 题目或语音要重做的话，回到草稿，等 HR 再点一次准备
    let back_to_draft = (content_changed || voice_changed) && row.plan.is_some();
    let status = if back_to_draft {
        InterviewStatus::Draft
    } else {
        row.status
    };

    let mut tx = deps.ctx.db.begin().await?;
    let updated = sqlx::query(
        "UPDATE interviews SET config = $3, resume_id = $4, voice_id = $5, status = $6, \
             plan = CASE WHEN $7 THEN NULL ELSE plan END, \
             prepare_error = CASE WHEN $6 = 'draft' THEN NULL ELSE prepare_error END \
         WHERE id = $1 AND status = $2",
    )
    .bind(id)
    .bind(row.status)
    .bind(serde_json::to_value(&config).map_err(ApiError::internal)?)
    .bind(resume_id)
    .bind(voice_id)
    .bind(status)
    .bind(content_changed)
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(ApiError::conflict("面试的状态刚刚变了，刷新后再试"));
    }
    if content_changed {
        sqlx::query(
            "DELETE FROM interview_utterances WHERE interview_id = $1 AND source = 'planned'",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
    } else if voice_changed {
        sqlx::query(
            "UPDATE interview_utterances SET synth_status = 'pending', synth_error = NULL \
             WHERE interview_id = $1 AND source = 'planned'",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Json(detail(&deps, id).await?))
}

/// 归档面试
///
/// 归档后列表里默认不显示，数据还在。正在准备或者正在面试的不能归档。
#[utoipa::path(
    delete,
    path = "/api/v1/interviews/{id}",
    tag = "interviews",
    params(("id" = Uuid, Path, description = "面试 ID")),
    responses(
        (status = 204, description = "归档了"),
        (status = 409, description = "正在准备或者正在面试", body = ErrorBody),
    )
)]
async fn archive_interview(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let row = load(&deps, id).await?;
    if matches!(
        row.status,
        InterviewStatus::Preparing | InterviewStatus::InProgress
    ) {
        return Err(ApiError::conflict("正在准备或者正在面试，先取消再归档"));
    }
    sqlx::query("UPDATE interviews SET archived_at = now() WHERE id = $1 AND archived_at IS NULL")
        .bind(id)
        .execute(&deps.ctx.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 开始准备
///
/// 出题并预先合成面试官的语音，在后台跑，一般一两分钟。接口马上返回，状态变成 `preparing`，
/// 之后轮询详情：成功是 `ready`，失败是 `prepare_failed`（原因在 `prepare_error`）。
/// 已经有题目时默认不重新出，只补上没合成好的语音；要重新出题传 `regenerate_plan: true`。
#[utoipa::path(
    post,
    path = "/api/v1/interviews/{id}/prepare",
    tag = "interviews",
    params(("id" = Uuid, Path, description = "面试 ID")),
    request_body(content = PrepareRequest, description = "可以不传"),
    responses(
        (status = 202, description = "已经开始准备", body = InterviewDetail),
        (status = 409, description = "现在的状态不能准备", body = ErrorBody),
        (status = 503, description = "大模型没配置，出不了题", body = ErrorBody),
    )
)]
async fn prepare_interview(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<InterviewDetail>)> {
    let request: PrepareRequest = if body.iter().all(u8::is_ascii_whitespace) {
        PrepareRequest::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|e| ApiError::bad_request(format!("请求体不是合法的 JSON：{e}")))?
    };
    let row = load(&deps, id).await?;
    ensure_editable(row.status)?;
    let needs_plan = request.regenerate_plan || row.plan.is_none();
    if needs_plan && deps.llm.is_none() {
        return Err(ApiError::unavailable(
            "大模型还没配置（环境变量 DEEPSEEK_API_KEY），出不了题",
        ));
    }

    let mut tx = deps.ctx.db.begin().await?;
    if !store::set_status(&mut tx, id, &EDITABLE, InterviewStatus::Preparing, None).await? {
        return Err(ApiError::conflict("面试的状态刚刚变了，刷新后再试"));
    }
    if request.regenerate_plan {
        sqlx::query("UPDATE interviews SET plan = NULL WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "DELETE FROM interview_utterances WHERE interview_id = $1 AND source = 'planned'",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    prepare::enqueue(&mut tx, task_kind::INTERVIEW_PREPARE, id).await?;
    tx.commit().await?;
    Ok((StatusCode::ACCEPTED, Json(detail(&deps, id).await?)))
}

/// 取消面试
///
/// 还没结束的面试都可以取消。已经取消的再取消一次也算成功。
#[utoipa::path(
    post,
    path = "/api/v1/interviews/{id}/cancel",
    tag = "interviews",
    params(("id" = Uuid, Path, description = "面试 ID")),
    responses(
        (status = 200, description = "取消了", body = InterviewDetail),
        (status = 409, description = "面试已经结束", body = ErrorBody),
    )
)]
async fn cancel_interview(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<InterviewDetail>> {
    let row = load(&deps, id).await?;
    if row.status != InterviewStatus::Cancelled {
        let open: Vec<InterviewStatus> = InterviewStatus::ALL
            .iter()
            .copied()
            .filter(|s| !s.is_terminal())
            .collect();
        let mut conn = deps.ctx.db.acquire().await?;
        if !store::set_status(&mut conn, id, &open, InterviewStatus::Cancelled, None).await? {
            return Err(ApiError::conflict("面试已经结束，不能取消"));
        }
    }
    Ok(Json(detail(&deps, id).await?))
}

async fn load_plan(row: &InterviewRow) -> ApiResult<InterviewPlan> {
    row.plan()
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("题目").with_code("plan_not_ready"))
}

/// 看题
///
/// 面试计划：开场白、题目和追问、过渡语、结束语。还没出题时是 404，code 是 `plan_not_ready`。
#[utoipa::path(
    get,
    path = "/api/v1/interviews/{id}/plan",
    tag = "interviews",
    params(("id" = Uuid, Path, description = "面试 ID")),
    responses(
        (status = 200, description = "面试计划", body = InterviewPlan),
        (status = 404, description = "没有这场面试，或者还没出题", body = ErrorBody),
    )
)]
async fn get_plan(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<InterviewPlan>> {
    let row = load(&deps, id).await?;
    Ok(Json(load_plan(&row).await?))
}

/// 改题
///
/// 传完整的面试计划（先 GET 下来再改）。只能在发链接之前改。保存后文字变了的话会重新合成语音，
/// 状态先变成 `preparing`，合成完回到 `ready`。
#[utoipa::path(
    put,
    path = "/api/v1/interviews/{id}/plan",
    tag = "interviews",
    params(("id" = Uuid, Path, description = "面试 ID")),
    request_body = InterviewPlan,
    responses(
        (status = 200, description = "保存了", body = InterviewPlan),
        (status = 404, description = "没有这场面试，或者还没出题", body = ErrorBody),
        (status = 409, description = "现在的状态不能改", body = ErrorBody),
        (status = 422, description = "计划不合要求，details 里按位置列出原因", body = ErrorBody),
    )
)]
async fn put_plan(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
    Json(mut plan): Json<InterviewPlan>,
) -> ApiResult<Json<InterviewPlan>> {
    let row = load(&deps, id).await?;
    ensure_editable(row.status)?;
    let old = load_plan(&row).await?;
    let config = row.config().map_err(ApiError::internal)?;

    // 出题过程的记录（模型、用量）不是 HR 能改的
    plan.generation = old.generation;
    plan.recompute_expected_duration(&config);
    let issues = plan.validate(&config);
    if !issues.is_empty() {
        let details: Map<String, Value> = issues
            .iter()
            .map(|issue| (issue.location.clone(), Value::String(issue.message.clone())))
            .collect();
        return Err(ApiError::validation(
            format!("面试计划有 {} 处问题", issues.len()),
            Value::Object(details),
        ));
    }

    let mut tx = deps.ctx.db.begin().await?;
    if !store::set_status(&mut tx, id, &[row.status], InterviewStatus::Preparing, None).await? {
        return Err(ApiError::conflict("面试的状态刚刚变了，刷新后再试"));
    }
    sqlx::query("UPDATE interviews SET plan = $2 WHERE id = $1")
        .bind(id)
        .bind(serde_json::to_value(&plan).map_err(ApiError::internal)?)
        .execute(&mut *tx)
        .await?;
    store::sync_utterances(&mut tx, id, &plan.utterances(), false).await?;
    prepare::enqueue(&mut tx, task_kind::INTERVIEW_SYNTHESIZE, id).await?;
    tx.commit().await?;
    Ok(Json(plan))
}

/// 面试官的每句话
///
/// 按面试的大致顺序列出面试官要说的话和语音合成的状态。
#[utoipa::path(
    get,
    path = "/api/v1/interviews/{id}/utterances",
    tag = "interviews",
    params(("id" = Uuid, Path, description = "面试 ID")),
    responses((status = 200, description = "每句话", body = Vec<UtteranceView>))
)]
async fn list_utterances(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<UtteranceView>>> {
    load(&deps, id).await?;
    Ok(Json(store::list_utterances(&deps.ctx.db, id).await?))
}

/// 试听
///
/// 返回这句话的音频（WAV），支持 `Range`。
#[utoipa::path(
    get,
    path = "/api/v1/interviews/{id}/utterances/{key}/audio",
    tag = "interviews",
    params(
        ("id" = Uuid, Path, description = "面试 ID"),
        ("key" = String, Path, description = "这句话的 key，例如 q1"),
    ),
    responses(
        (status = 200, description = "音频", content_type = "audio/wav"),
        (status = 404, description = "没有这句话，或者还没合成好", body = ErrorBody),
    )
)]
async fn utterance_audio(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path((id, key)): Path<(Uuid, String)>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let (blob_key, content_type) = store::utterance_audio(&deps.ctx.db, id, &key)
        .await?
        .ok_or_else(|| ApiError::not_found("这句话的音频"))?;
    serve_blob(
        deps.ctx.blobs.as_ref(),
        &blob_key,
        content_type.as_deref().unwrap_or("audio/wav"),
        &headers,
    )
    .await
}

/// 配置的默认值和取值范围
///
/// 给配置表单用：默认配置、各项允许的范围和建议的选项。
#[utoipa::path(
    get,
    path = "/api/v1/interview-config/defaults",
    tag = "interviews",
    responses((status = 200, description = "默认值和取值范围", body = ConfigDefaults))
)]
async fn config_defaults(_user: AdminUser) -> Json<ConfigDefaults> {
    let config = InterviewConfig::default();
    Json(ConfigDefaults {
        time_budget: config.time_budget(),
        config,
        limits: ConfigLimits::current(),
    })
}
