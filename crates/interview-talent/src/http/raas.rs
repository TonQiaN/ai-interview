//! 从 RAAS 浏览和导入岗位、候选人。
//!
//! 对 RAAS 只读：这里只调 `interview-raas` 的查询方法，写的只有我们自己的库。
//!
//! 导入是同步的：一次最多 [`MAX_IMPORT`] 条，每条各自成功或失败，结果逐条返回。
//! 候选人的简历导进来以后走和上传一样的路（`resume.parse` 任务），由大模型读。

use std::collections::HashSet;

use axum::extract::State;
use axum::http::StatusCode;
use bytes::Bytes;
use futures_util::{StreamExt, TryStreamExt, stream};
use interview_platform::auth::AdminUser;
use interview_platform::http::{ErrorBody, Json, Page, PageQuery, Path, Query};
use interview_platform::{ApiError, ApiResult};
use interview_raas::{
    ApplicationQuery, ApplicationSummary, CandidateDetail as RaasCandidateDetail, CandidateQuery,
    CandidateSummary, JobPostingQuery, JobPostingSummary, RaasClient, RaasError, ResumeRecord,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use super::upload::{self, Upload};
use crate::deps::Deps;
use crate::position::PositionDraft;
use crate::raas::{
    candidate_from_raas, has_resume_content, position_from_raas, position_status, resume_markdown,
};
use crate::store::candidates::{self, CandidateSource, DuplicateHint, NewCandidate};
use crate::store::positions::{self, NewPosition, PositionSource, PositionStatus};
use crate::store::resumes;
use crate::text_enum::text_enum;

/// 一次最多导入多少条。导入是同步的，太多了请求会等很久。
pub const MAX_IMPORT: usize = 20;
/// 同时向 RAAS 取几条。RAAS 是别人的生产系统，不要压它。
const CONCURRENCY: usize = 4;

pub(super) fn router() -> OpenApiRouter<Deps> {
    OpenApiRouter::new()
        .routes(routes!(raas_status))
        .routes(routes!(list_raas_positions))
        .routes(routes!(import_raas_positions))
        .routes(routes!(get_raas_position))
        .routes(routes!(list_raas_candidates))
        .routes(routes!(import_raas_candidates))
        .routes(routes!(get_raas_candidate))
}

// ───────────────────────── 出错 ─────────────────────────

/// 把访问 RAAS 的错误换成接口的错误。原始错误只进日志（它本身不带候选人信息）。
pub(crate) fn raas_error(error: RaasError) -> ApiError {
    let response = match &error {
        RaasError::MissingConfig(_) | RaasError::InvalidBaseUrl(_) => {
            ApiError::unavailable("RAAS 的地址或账号没有配好，请联系管理员")
                .with_code("raas_unavailable")
        }
        RaasError::LoginRejected { .. }
        | RaasError::LoginDisabled
        | RaasError::LoginWithoutToken => ApiError::unavailable(
            "RAAS 登录不上，请联系管理员核对账号。为了不锁账号，核对并重启服务之前不会再试",
        )
        .with_code("raas_unavailable"),
        RaasError::Http { status: 404, .. } => ApiError::not_found("RAAS 里的这条记录"),
        _ => ApiError::new(
            StatusCode::BAD_GATEWAY,
            "raas_error",
            "RAAS 暂时访问不了，请稍后再试",
        ),
    };
    response.with_source(error)
}

/// 这个错误是不是说明整个 RAAS 都用不了（没配好、登录不上）。这种情况不用一条一条试下去。
fn is_fatal(error: &RaasError) -> bool {
    matches!(
        error,
        RaasError::MissingConfig(_)
            | RaasError::InvalidBaseUrl(_)
            | RaasError::LoginRejected { .. }
            | RaasError::LoginDisabled
            | RaasError::LoginWithoutToken
    )
}

/// 导入某一条时 RAAS 出了错：整个 RAAS 用不了就让整个请求失败，否则只算这一条失败，返回给人看的原因。
fn item_failure(error: RaasError, what: &str) -> ApiResult<String> {
    if is_fatal(&error) {
        return Err(raas_error(error));
    }
    if error.is_not_found() {
        return Ok(format!("RAAS 里没有这{what}"));
    }
    tracing::warn!(%error, "从 RAAS 取数据失败");
    Ok("RAAS 暂时访问不了，请稍后再试".to_owned())
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .is_some_and(sqlx::error::DatabaseError::is_unique_violation)
}

fn total(total: u64) -> i64 {
    i64::try_from(total).unwrap_or(i64::MAX)
}

// ───────────────────────── 连接状态 ─────────────────────────

/// RAAS 的连接状态。
#[derive(Debug, Serialize, ToSchema)]
pub struct RaasStatus {
    /// 有没有配置 RAAS 的地址和账号
    pub configured: bool,
    /// 现在连不连得上
    pub reachable: bool,
    /// 连不上时的原因
    pub message: Option<String>,
}

/// RAAS 是否连得上
///
/// 真的向 RAAS 发一次查询（只取一条岗位）。没配置或者连不上都返回 200，看 `configured` 和 `reachable`。
#[utoipa::path(
    get,
    path = "/api/v1/raas/status",
    tag = "raas",
    responses((status = 200, description = "连接状态", body = RaasStatus))
)]
async fn raas_status(State(deps): State<Deps>, _user: AdminUser) -> Json<RaasStatus> {
    let Some(raas) = deps.raas.as_ref() else {
        return Json(RaasStatus {
            configured: false,
            reachable: false,
            message: Some("还没有配置 RAAS 的地址和账号".to_owned()),
        });
    };
    let probe = JobPostingQuery {
        page_size: Some(1),
        ..JobPostingQuery::default()
    };
    match raas.list_job_postings(&probe).await {
        Ok(_) => Json(RaasStatus {
            configured: true,
            reachable: true,
            message: None,
        }),
        Err(error) => {
            tracing::warn!(%error, "RAAS 连不上");
            Json(RaasStatus {
                configured: true,
                reachable: false,
                message: Some(raas_error(error).message().to_owned()),
            })
        }
    }
}

// ───────────────────────── 岗位 ─────────────────────────

/// RAAS 岗位列表的筛选条件。和分页参数一起用（`q` 是关键词，`sort` 不起作用）。
#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct RaasPositionFilter {
    /// RAAS 里的岗位状态，原样传过去。常用 `recruiting`（招聘中）
    pub status: Option<String>,
    /// 工作城市
    pub city: Option<String>,
}

/// RAAS 里的一个岗位（列表里的一行）。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RaasPositionItem {
    /// RAAS 里的岗位编号。看详情和导入都用它
    pub external_ref: String,
    /// RAAS 里的需求编号。看这个岗位下投递的候选人时用它（`job_requisition_ref`）
    pub job_requisition_ref: Option<String>,
    pub title: Option<String>,
    /// 客户名称
    pub client_name: Option<String>,
    pub department: Option<String>,
    pub cities: Vec<String>,
    /// 招几个人
    pub headcount: Option<f64>,
    /// 工作年限要求
    pub work_years: Option<String>,
    /// 学历要求
    pub degree_requirement: Option<String>,
    pub salary_range: Option<String>,
    /// 级别
    pub expected_level: Option<String>,
    /// 换算成我们这边的岗位状态
    pub status: PositionStatus,
    /// RAAS 里上次修改的时间
    pub updated_at: Option<String>,
    /// 已经导入过的话，是我们这边的岗位 id
    pub imported_position_id: Option<Uuid>,
}

impl RaasPositionItem {
    fn new(summary: JobPostingSummary, imported_position_id: Option<Uuid>) -> Self {
        Self {
            status: position_status(
                summary.publish_status.as_deref(),
                summary.requirement_status.as_deref(),
            ),
            external_ref: summary.job_posting_id,
            job_requisition_ref: summary.job_requisition_ref,
            title: summary.title,
            client_name: summary.client_name,
            department: summary.department,
            cities: summary.cities,
            headcount: summary.headcount,
            work_years: summary.work_years,
            degree_requirement: summary.degree_requirement,
            salary_range: summary.salary_range,
            expected_level: summary.expected_level,
            updated_at: summary.updated_at,
            imported_position_id,
        }
    }
}

/// RAAS 里一个岗位的详情：导入以后会是什么样。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RaasPositionPreview {
    pub external_ref: String,
    pub job_requisition_ref: Option<String>,
    pub status: PositionStatus,
    /// 导入后岗位的内容
    pub draft: PositionDraft,
    /// 已经导入过的话，是我们这边的岗位 id
    pub imported_position_id: Option<Uuid>,
}

/// 导入请求。
#[derive(Debug, Deserialize, ToSchema)]
pub struct RaasImportRequest {
    /// 要导入的 RAAS 编号（列表里的 `external_ref`），一次 1 到 20 个
    pub refs: Vec<String>,
    /// 已经导入过的要不要用 RAAS 里最新的内容更新。默认不更新
    #[serde(default)]
    pub refresh: bool,
}

impl RaasImportRequest {
    /// 去掉空的和重复的，检查条数。
    fn cleaned_refs(&self) -> ApiResult<Vec<String>> {
        let mut seen = HashSet::new();
        let refs: Vec<String> = self
            .refs
            .iter()
            .map(|external_ref| external_ref.trim())
            .filter(|external_ref| !external_ref.is_empty())
            .filter(|external_ref| seen.insert(*external_ref))
            .map(str::to_owned)
            .collect();
        if refs.is_empty() {
            return Err(ApiError::invalid_field("refs", "请至少选一条"));
        }
        if refs.len() > MAX_IMPORT {
            return Err(ApiError::invalid_field(
                "refs",
                format!("一次最多导入 {MAX_IMPORT} 条"),
            ));
        }
        Ok(refs)
    }
}

text_enum! {
    /// 导入一条的结果。
    pub enum ImportOutcome {
        /// 新建了
        Created => "created",
        /// 以前导入过，这次没动
        AlreadyImported => "already_imported",
        /// 以前导入过，这次用 RAAS 的内容更新了
        Refreshed => "refreshed",
        /// 以前导入过但归档了，这次恢复并更新了
        Restored => "restored",
        /// 没导成，原因见 `message`
        Failed => "failed",
    }
}

/// 导入一个岗位的结果。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PositionImportItem {
    pub external_ref: String,
    pub outcome: ImportOutcome,
    /// 我们这边的岗位 id。没导成时是空
    pub position_id: Option<Uuid>,
    pub title: Option<String>,
    /// 没导成的原因
    pub message: Option<String>,
}

/// 导入岗位的结果，顺序和请求里的 `refs` 一致。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PositionImportResult {
    pub items: Vec<PositionImportItem>,
}

/// RAAS 岗位列表
///
/// 实时去 RAAS 查。`q` 是关键词，`imported_position_id` 有值表示已经导入过。
#[utoipa::path(
    get,
    path = "/api/v1/raas/positions",
    tag = "raas",
    params(PageQuery, RaasPositionFilter),
    responses(
        (status = 200, description = "一页 RAAS 岗位", body = Page<RaasPositionItem>),
        (status = 502, description = "RAAS 暂时访问不了", body = ErrorBody),
        (status = 503, description = "RAAS 没配置或登录不上", body = ErrorBody),
    )
)]
async fn list_raas_positions(
    State(deps): State<Deps>,
    _user: AdminUser,
    Query(page): Query<PageQuery>,
    Query(filter): Query<RaasPositionFilter>,
) -> ApiResult<Json<Page<RaasPositionItem>>> {
    let raas = deps.require_raas()?;
    let found = raas
        .list_job_postings(&JobPostingQuery {
            search: page.keyword().map(str::to_owned),
            status: non_empty(filter.status),
            city: non_empty(filter.city),
            page: Some(page.page()),
            page_size: Some(page.page_size()),
        })
        .await
        .map_err(raas_error)?;
    let refs: Vec<String> = found
        .items
        .iter()
        .map(|summary| summary.job_posting_id.clone())
        .collect();
    let imported = positions::imported_ids(&deps.ctx.db, PositionSource::Raas, &refs).await?;
    let items = found
        .items
        .into_iter()
        .map(|summary| {
            let id = imported.get(&summary.job_posting_id).copied();
            RaasPositionItem::new(summary, id)
        })
        .collect();
    Ok(Json(Page::new(items, total(found.total), &page)))
}

/// RAAS 岗位详情
///
/// 导入之前先看看：`draft` 是导入后岗位的内容。
#[utoipa::path(
    get,
    path = "/api/v1/raas/positions/{ref}",
    tag = "raas",
    params(("ref" = String, Path, description = "RAAS 里的岗位编号")),
    responses(
        (status = 200, description = "岗位详情", body = RaasPositionPreview),
        (status = 404, description = "RAAS 里没有这个岗位", body = ErrorBody),
        (status = 502, description = "RAAS 暂时访问不了", body = ErrorBody),
        (status = 503, description = "RAAS 没配置或登录不上", body = ErrorBody),
    )
)]
async fn get_raas_position(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(external_ref): Path<String>,
) -> ApiResult<Json<RaasPositionPreview>> {
    let raas = deps.require_raas()?;
    let detail = raas.job_posting(&external_ref).await.map_err(raas_error)?;
    let job_requisition_ref = detail.summary.job_requisition_ref.clone();
    let imported = position_from_raas(&detail);
    let existing = positions::imported_ids(
        &deps.ctx.db,
        PositionSource::Raas,
        std::slice::from_ref(&imported.external_ref),
    )
    .await?;
    Ok(Json(RaasPositionPreview {
        imported_position_id: existing.get(&imported.external_ref).copied(),
        external_ref: imported.external_ref,
        job_requisition_ref,
        status: imported.status,
        draft: imported.draft,
    }))
}

/// 导入 RAAS 岗位
///
/// 一次最多 20 个，每个各自成功或失败，看 `items[].outcome`。已经导入过的默认不动；
/// `refresh` 为真时用 RAAS 里最新的内容覆盖（在我们这边改过的会被盖掉）。归档过的会恢复。
#[utoipa::path(
    post,
    path = "/api/v1/raas/positions/import",
    tag = "raas",
    request_body = RaasImportRequest,
    responses(
        (status = 200, description = "逐条的导入结果", body = PositionImportResult),
        (status = 422, description = "没选或者选得太多", body = ErrorBody),
        (status = 503, description = "RAAS 没配置或登录不上", body = ErrorBody),
    )
)]
async fn import_raas_positions(
    State(deps): State<Deps>,
    user: AdminUser,
    Json(request): Json<RaasImportRequest>,
) -> ApiResult<Json<PositionImportResult>> {
    let raas = deps.require_raas()?;
    let refs = request.cleaned_refs()?;
    let refresh = request.refresh;
    let (deps, raas) = (&deps, raas.as_ref());
    let items = stream::iter(refs)
        .map(|external_ref| async move {
            import_position(deps, raas, user.id, external_ref, refresh).await
        })
        .buffered(CONCURRENCY)
        .try_collect()
        .await?;
    Ok(Json(PositionImportResult { items }))
}

async fn import_position(
    deps: &Deps,
    raas: &RaasClient,
    user_id: Uuid,
    external_ref: String,
    refresh: bool,
) -> ApiResult<PositionImportItem> {
    let db = &deps.ctx.db;
    let existing = positions::get_by_external_ref(db, PositionSource::Raas, &external_ref).await?;
    if let Some(position) = &existing
        && position.archived_at.is_none()
        && !refresh
    {
        return Ok(PositionImportItem {
            external_ref,
            outcome: ImportOutcome::AlreadyImported,
            position_id: Some(position.id),
            title: Some(position.title.clone()),
            message: None,
        });
    }

    let detail = match raas.job_posting(&external_ref).await {
        Ok(detail) => detail,
        Err(error) => {
            return Ok(PositionImportItem {
                external_ref,
                outcome: ImportOutcome::Failed,
                position_id: None,
                title: None,
                message: Some(item_failure(error, "个岗位")?),
            });
        }
    };
    let imported = position_from_raas(&detail);
    let new = NewPosition {
        title: imported.draft.title,
        department: imported.draft.department,
        location: imported.draft.location,
        employment_type: imported.draft.employment_type,
        seniority: imported.draft.seniority,
        description: imported.draft.description,
        requirements: imported.draft.requirements,
        extra: imported.draft.extra,
        status: imported.status,
        source: PositionSource::Raas,
        external_ref: Some(external_ref.clone()),
        created_by: Some(user_id),
    };

    let (position, outcome) = match existing {
        Some(existing) => {
            let outcome = if existing.archived_at.is_some() {
                ImportOutcome::Restored
            } else {
                ImportOutcome::Refreshed
            };
            (
                positions::overwrite_from_source(db, existing.id, new).await?,
                outcome,
            )
        }
        None => match positions::insert(db, new).await {
            Ok(position) => (position, ImportOutcome::Created),
            // 另一个请求刚好也在导入同一个岗位，抢先了一步
            Err(error) if is_unique_violation(&error) => {
                let position =
                    positions::get_by_external_ref(db, PositionSource::Raas, &external_ref)
                        .await?
                        .ok_or(error)?;
                (position, ImportOutcome::AlreadyImported)
            }
            Err(error) => return Err(error.into()),
        },
    };
    Ok(PositionImportItem {
        external_ref,
        outcome,
        position_id: Some(position.id),
        title: Some(position.title),
        message: None,
    })
}

// ───────────────────────── 候选人 ─────────────────────────

/// RAAS 候选人列表的筛选条件。和分页参数一起用（`q` 是关键词，`sort` 不起作用）。
#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct RaasCandidateFilter {
    /// 只看有（或没有）简历的
    pub has_resume: Option<bool>,
    /// 只看投了这个需求的候选人。值是岗位的 `job_requisition_ref`。给了它，`q` 和 `has_resume` 不起作用
    pub job_requisition_ref: Option<String>,
}

/// RAAS 里的一位候选人（列表里的一行）。不带手机和邮箱。
#[derive(Clone, Serialize, ToSchema)]
pub struct RaasCandidateItem {
    /// RAAS 里的候选人编号。看详情和导入都用它
    pub external_ref: String,
    pub name: Option<String>,
    /// 最高学历
    pub degree: Option<String>,
    pub graduation_school: Option<String>,
    pub major: Option<String>,
    pub current_company: Option<String>,
    pub current_title: Option<String>,
    pub current_city: Option<String>,
    /// 工作年限
    pub experience_years: Option<f64>,
    pub skills: Vec<String>,
    /// 最近投的岗位
    pub latest_application_job_title: Option<String>,
    /// 按需求查的时候：这次投递的状态
    pub application_status: Option<String>,
    /// 按需求查的时候：这次投递走到哪一步
    pub application_stage: Option<String>,
    /// 按需求查的时候：RAAS 给的人岗匹配分（0 到 100）
    pub matching_score: Option<f64>,
    /// 已经导入过的话，是我们这边的候选人 id
    pub imported_candidate_id: Option<Uuid>,
}

impl RaasCandidateItem {
    fn from_summary(summary: CandidateSummary) -> Self {
        Self {
            external_ref: summary.candidate_id,
            name: summary.name,
            degree: summary.degree,
            graduation_school: summary.graduation_school,
            major: summary.major,
            current_company: summary.current_company,
            current_title: summary.current_title,
            current_city: summary.current_city,
            experience_years: summary.experience_years,
            skills: summary.skills,
            latest_application_job_title: summary.latest_application_job_title,
            application_status: None,
            application_stage: None,
            matching_score: None,
            imported_candidate_id: None,
        }
    }

    /// 投递记录里没有候选人编号的（数据不全）导不了，不列出来。
    fn from_application(application: ApplicationSummary) -> Option<Self> {
        Some(Self {
            external_ref: application.candidate_id?,
            name: application.candidate_name,
            degree: application.candidate_education,
            graduation_school: None,
            major: None,
            current_company: None,
            current_title: None,
            current_city: application.candidate_city,
            experience_years: None,
            skills: Vec::new(),
            latest_application_job_title: None,
            application_status: application.status,
            application_stage: application.stage,
            matching_score: application.effective_matching_score,
            imported_candidate_id: None,
        })
    }
}

/// 候选人在 RAAS 里的一次投递。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RaasApplicationBrief {
    /// 投的需求编号
    pub job_requisition_ref: Option<String>,
    pub job_title: Option<String>,
    pub status: Option<String>,
    pub stage: Option<String>,
}

/// RAAS 里一位候选人的详情。不带手机和邮箱：导入以后在我们这边看。
#[derive(Clone, Serialize, ToSchema)]
pub struct RaasCandidatePreview {
    pub external_ref: String,
    pub name: Option<String>,
    pub degree: Option<String>,
    pub graduation_school: Option<String>,
    pub experience_years: Option<f64>,
    pub current_city: Option<String>,
    pub current_company: Option<String>,
    pub current_title: Option<String>,
    /// 想找的职位
    pub job_intention: Option<String>,
    pub skills: Vec<String>,
    /// RAAS 里有没有简历（原始文件或者解析出来的内容）。没有的话导入后只有基本信息
    pub has_resume: bool,
    pub applications: Vec<RaasApplicationBrief>,
    /// 已经导入过的话，是我们这边的候选人 id
    pub imported_candidate_id: Option<Uuid>,
}

text_enum! {
    /// 导入候选人时简历的处理结果。
    pub enum ResumeImportOutcome {
        /// 导进来了，已经排队等大模型读
        Queued => "queued",
        /// 和已经有的一份内容一样，没有重复导入
        Unchanged => "unchanged",
        /// RAAS 里没有这位候选人的简历
        None => "none",
        /// 取简历的时候 RAAS 出错了。候选人已经建好，可以稍后带 `refresh` 再导一次
        Failed => "failed",
    }
}

/// 导入一位候选人的结果。
#[derive(Clone, Serialize, ToSchema)]
pub struct CandidateImportItem {
    pub external_ref: String,
    pub outcome: ImportOutcome,
    /// 我们这边的候选人 id。没导成时是空
    pub candidate_id: Option<Uuid>,
    pub name: Option<String>,
    /// 简历怎么样了。这次没碰简历（没导成，或者以前导入过这次没动）时是空
    pub resume: Option<ResumeImportOutcome>,
    /// 新导进来的那份简历的 id
    pub resume_id: Option<Uuid>,
    /// 库里手机号或邮箱一样的其他候选人，可能是同一个人
    pub possible_duplicates: Vec<DuplicateHint>,
    /// 没导成的原因
    pub message: Option<String>,
}

impl CandidateImportItem {
    fn untouched(external_ref: String, outcome: ImportOutcome) -> Self {
        Self {
            external_ref,
            outcome,
            candidate_id: None,
            name: None,
            resume: None,
            resume_id: None,
            possible_duplicates: Vec::new(),
            message: None,
        }
    }
}

/// 导入候选人的结果，顺序和请求里的 `refs` 一致。
#[derive(Clone, Serialize, ToSchema)]
pub struct CandidateImportResult {
    pub items: Vec<CandidateImportItem>,
}

/// RAAS 候选人列表
///
/// 实时去 RAAS 查。给了 `job_requisition_ref` 就列投了那个需求的候选人，否则在整个人才库里按关键词找。
/// 列表里不带手机和邮箱。`imported_candidate_id` 有值表示已经导入过。
#[utoipa::path(
    get,
    path = "/api/v1/raas/candidates",
    tag = "raas",
    params(PageQuery, RaasCandidateFilter),
    responses(
        (status = 200, description = "一页 RAAS 候选人", body = Page<RaasCandidateItem>),
        (status = 502, description = "RAAS 暂时访问不了", body = ErrorBody),
        (status = 503, description = "RAAS 没配置或登录不上", body = ErrorBody),
    )
)]
async fn list_raas_candidates(
    State(deps): State<Deps>,
    _user: AdminUser,
    Query(page): Query<PageQuery>,
    Query(filter): Query<RaasCandidateFilter>,
) -> ApiResult<Json<Page<RaasCandidateItem>>> {
    let raas = deps.require_raas()?;
    let (mut items, found_total) = match non_empty(filter.job_requisition_ref) {
        Some(job_requisition_ref) => {
            let found = raas
                .list_applications(
                    &job_requisition_ref,
                    &ApplicationQuery {
                        page: Some(page.page()),
                        page_size: Some(page.page_size()),
                        ..ApplicationQuery::default()
                    },
                )
                .await
                .map_err(raas_error)?;
            // 同一个人可能对同一个需求有不止一条投递记录，只列一次
            let mut seen = HashSet::new();
            let items: Vec<RaasCandidateItem> = found
                .items
                .into_iter()
                .filter_map(RaasCandidateItem::from_application)
                .filter(|item| seen.insert(item.external_ref.clone()))
                .collect();
            (items, found.total)
        }
        None => {
            let found = raas
                .list_candidates(&CandidateQuery {
                    search: page.keyword().map(str::to_owned),
                    has_resume: filter.has_resume,
                    page: Some(page.page()),
                    page_size: Some(page.page_size()),
                    ..CandidateQuery::default()
                })
                .await
                .map_err(raas_error)?;
            let items = found
                .items
                .into_iter()
                .map(RaasCandidateItem::from_summary)
                .collect();
            (items, found.total)
        }
    };
    let refs: Vec<String> = items.iter().map(|item| item.external_ref.clone()).collect();
    let imported = candidates::imported_ids(&deps.ctx.db, CandidateSource::Raas, &refs).await?;
    for item in &mut items {
        item.imported_candidate_id = imported.get(&item.external_ref).copied();
    }
    Ok(Json(Page::new(items, total(found_total), &page)))
}

/// RAAS 候选人详情
///
/// 导入之前先看看。不带手机和邮箱。
#[utoipa::path(
    get,
    path = "/api/v1/raas/candidates/{ref}",
    tag = "raas",
    params(("ref" = String, Path, description = "RAAS 里的候选人编号")),
    responses(
        (status = 200, description = "候选人详情", body = RaasCandidatePreview),
        (status = 404, description = "RAAS 里没有这位候选人", body = ErrorBody),
        (status = 502, description = "RAAS 暂时访问不了", body = ErrorBody),
        (status = 503, description = "RAAS 没配置或登录不上", body = ErrorBody),
    )
)]
async fn get_raas_candidate(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(external_ref): Path<String>,
) -> ApiResult<Json<RaasCandidatePreview>> {
    let raas = deps.require_raas()?;
    let detail = raas.candidate(&external_ref).await.map_err(raas_error)?;
    let imported = candidates::imported_ids(
        &deps.ctx.db,
        CandidateSource::Raas,
        std::slice::from_ref(&detail.candidate_id),
    )
    .await?;
    let has_resume = detail.latest_resume_id.is_some()
        || detail.resume_original_filename.is_some()
        || has_resume_content(&detail, None);
    Ok(Json(RaasCandidatePreview {
        imported_candidate_id: imported.get(&detail.candidate_id).copied(),
        has_resume,
        applications: detail
            .applications
            .into_iter()
            .map(|application| RaasApplicationBrief {
                job_requisition_ref: application.job_requisition_id,
                job_title: application.job_title,
                status: application.status,
                stage: application.stage,
            })
            .collect(),
        external_ref: detail.candidate_id,
        name: detail.name,
        degree: detail.degree,
        graduation_school: detail.graduation_school,
        experience_years: detail.experience_years,
        current_city: detail.current_city,
        current_company: detail.current_company,
        current_title: detail.current_title,
        job_intention: detail.job_intention,
        skills: detail.skills,
    }))
}

/// 导入 RAAS 候选人
///
/// 一次最多 20 位，每位各自成功或失败，看 `items[].outcome`。基本信息直接建档；简历优先取 RAAS 里的
/// 原始文件，没有原始文件（或者是 Word 这类读不了的）就用 RAAS 的解析结果拼一份文字版，
/// 两种都交给后台的 `resume.parse` 任务让大模型读，读完自动生成画像。
///
/// 已经导入过的默认不动。`refresh` 为真时：基本信息只补空的（在我们这边改过的不动），
/// RAAS 里有新简历就导进来设成主简历。归档过的会恢复。
#[utoipa::path(
    post,
    path = "/api/v1/raas/candidates/import",
    tag = "raas",
    request_body = RaasImportRequest,
    responses(
        (status = 200, description = "逐条的导入结果", body = CandidateImportResult),
        (status = 422, description = "没选或者选得太多", body = ErrorBody),
        (status = 503, description = "RAAS 没配置或登录不上", body = ErrorBody),
    )
)]
async fn import_raas_candidates(
    State(deps): State<Deps>,
    user: AdminUser,
    Json(request): Json<RaasImportRequest>,
) -> ApiResult<Json<CandidateImportResult>> {
    let raas = deps.require_raas()?;
    let refs = request.cleaned_refs()?;
    let refresh = request.refresh;
    let (deps, raas) = (&deps, raas.as_ref());
    let mut items: Vec<CandidateImportItem> = stream::iter(refs)
        .map(|external_ref| async move {
            import_candidate(deps, raas, user.id, external_ref, refresh).await
        })
        .buffered(CONCURRENCY)
        .try_collect()
        .await?;
    // 查重放在全部导完之后：几位候选人是同时导的，边导边查的话，
    // 同一批里互相重复的两个人谁也看不到对方
    for item in &mut items {
        let touched = matches!(
            item.outcome,
            ImportOutcome::Created | ImportOutcome::Refreshed | ImportOutcome::Restored
        );
        let Some(candidate_id) = item.candidate_id.filter(|_| touched) else {
            continue;
        };
        if let Some(candidate) = candidates::get(&deps.ctx.db, candidate_id).await? {
            item.possible_duplicates = candidates::find_duplicates(
                &deps.ctx.db,
                Some(candidate.id),
                candidate.email.as_deref(),
                candidate.phone.as_deref(),
            )
            .await?;
        }
    }
    Ok(Json(CandidateImportResult { items }))
}

/// 从 RAAS 拿到的简历。
enum ResumeSource {
    /// 原始文件
    File(Upload),
    /// 没有能用的原始文件，用 RAAS 的解析结果拼的文字版
    Text(String),
    /// RAAS 里没有简历
    Missing,
    /// 取的时候出错了
    Failed,
}

async fn import_candidate(
    deps: &Deps,
    raas: &RaasClient,
    user_id: Uuid,
    external_ref: String,
    refresh: bool,
) -> ApiResult<CandidateImportItem> {
    let db = &deps.ctx.db;
    let existing =
        candidates::get_by_external_ref(db, CandidateSource::Raas, &external_ref).await?;
    if let Some(candidate) = &existing
        && candidate.archived_at.is_none()
        && !refresh
    {
        return Ok(CandidateImportItem {
            candidate_id: Some(candidate.id),
            name: Some(candidate.name.clone()),
            ..CandidateImportItem::untouched(external_ref, ImportOutcome::AlreadyImported)
        });
    }

    let detail = match raas.candidate(&external_ref).await {
        Ok(detail) => detail,
        Err(error) => {
            return Ok(CandidateImportItem {
                message: Some(item_failure(error, "位候选人")?),
                ..CandidateImportItem::untouched(external_ref, ImportOutcome::Failed)
            });
        }
    };
    // 先把要联网的事做完，再开事务
    let source = fetch_resume(raas, &detail).await?;

    let imported = candidate_from_raas(&detail);
    let new = NewCandidate {
        name: imported.name,
        email: imported.email,
        phone: imported.phone,
        current_title: imported.current_title,
        current_company: imported.current_company,
        location: imported.location,
        notes: None,
        source: CandidateSource::Raas,
        external_ref: Some(external_ref.clone()),
        created_by: Some(user_id),
    };

    let mut tx = db.begin().await?;
    let (candidate, outcome) = match &existing {
        Some(existing) => {
            let outcome = if existing.archived_at.is_some() {
                ImportOutcome::Restored
            } else {
                ImportOutcome::Refreshed
            };
            (
                candidates::fill_from_source(&mut tx, existing.id, &new).await?,
                outcome,
            )
        }
        None => match candidates::insert(&mut tx, new).await {
            Ok(candidate) => (candidate, ImportOutcome::Created),
            // 另一个请求刚好也在导入同一位候选人，抢先了一步
            Err(error) if is_unique_violation(&error) => {
                drop(tx);
                let candidate =
                    candidates::get_by_external_ref(db, CandidateSource::Raas, &external_ref)
                        .await?
                        .ok_or(error)?;
                return Ok(CandidateImportItem {
                    candidate_id: Some(candidate.id),
                    name: Some(candidate.name),
                    ..CandidateImportItem::untouched(external_ref, ImportOutcome::AlreadyImported)
                });
            }
            Err(error) => return Err(error.into()),
        },
    };

    // RAAS 里没填姓名的，名字是占位的，等简历读出来再用简历上的。
    // 看的是候选人现在的名字：以前导入过、HR 已经把名字填上的，不能再被简历上的换掉
    let adopt_name = upload::is_placeholder_name(&candidate, None);

    let (resume, resume_id) = match source {
        ResumeSource::File(upload) => {
            if resumes::find_by_sha256(&mut *tx, candidate.id, &upload.sha256)
                .await?
                .is_some()
            {
                (ResumeImportOutcome::Unchanged, None)
            } else {
                let resume =
                    upload::store_resume(deps, &mut tx, candidate.id, upload, adopt_name).await?;
                (ResumeImportOutcome::Queued, Some(resume.id))
            }
        }
        ResumeSource::Text(text) => {
            match upload::store_text_resume(&mut tx, candidate.id, text, adopt_name).await? {
                Some(resume) => (ResumeImportOutcome::Queued, Some(resume.id)),
                None => (ResumeImportOutcome::Unchanged, None),
            }
        }
        ResumeSource::Missing => (ResumeImportOutcome::None, None),
        ResumeSource::Failed => (ResumeImportOutcome::Failed, None),
    };
    tx.commit().await?;

    Ok(CandidateImportItem {
        external_ref,
        outcome,
        candidate_id: Some(candidate.id),
        name: Some(candidate.name),
        resume: Some(resume),
        resume_id,
        // 全部导完之后由调用方统一查
        possible_duplicates: Vec::new(),
        message: None,
    })
}

/// 取候选人的简历：先要原始文件，不行再用 RAAS 的解析结果。
async fn fetch_resume(raas: &RaasClient, detail: &RaasCandidateDetail) -> ApiResult<ResumeSource> {
    let candidate_id = detail.candidate_id.as_str();
    match raas.candidate_resume_file(candidate_id, None).await {
        Ok(file) => {
            let file_name = detail.resume_original_filename.as_deref();
            // Word 文档、空文件、超过大小上限的原件我们读不了，退回去用解析结果
            if let Ok(upload) = Upload::from_bytes(file_name, Bytes::from(file.bytes)) {
                return Ok(ResumeSource::File(upload));
            }
        }
        Err(error) if error.is_not_found() => {}
        Err(error) => {
            item_failure(error, "份简历")?;
            return Ok(ResumeSource::Failed);
        }
    }

    let records = match raas.candidate_resumes(candidate_id).await {
        Ok(records) => records,
        Err(error) if error.is_not_found() => Vec::new(),
        Err(error) => {
            item_failure(error, "份简历")?;
            return Ok(ResumeSource::Failed);
        }
    };
    let record = latest_record(&records);
    if has_resume_content(detail, record) {
        Ok(ResumeSource::Text(resume_markdown(detail, record)))
    } else {
        Ok(ResumeSource::Missing)
    }
}

/// 最新的一份简历版本：有标了「最新」的用它，没有就用第一份。
fn latest_record(records: &[ResumeRecord]) -> Option<&ResumeRecord> {
    records
        .iter()
        .find(|record| record.is_latest)
        .or_else(|| records.first())
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(refs: &[&str]) -> RaasImportRequest {
        RaasImportRequest {
            refs: refs.iter().map(|r| (*r).to_owned()).collect(),
            refresh: false,
        }
    }

    #[test]
    fn refs_are_trimmed_and_deduplicated() {
        let refs = request(&[" jp-1 ", "jp-2", "", "jp-1"])
            .cleaned_refs()
            .expect("合法");
        assert_eq!(refs, ["jp-1", "jp-2"]);
    }

    #[test]
    fn empty_or_too_many_refs_are_rejected() {
        let empty = request(&["  "]).cleaned_refs().unwrap_err();
        assert_eq!(empty.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let many: Vec<String> = (0..=MAX_IMPORT).map(|n| format!("jp-{n}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        let too_many = request(&many).cleaned_refs().unwrap_err();
        assert_eq!(too_many.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(too_many.message().contains("20"));
    }

    #[test]
    fn login_trouble_is_503_and_other_trouble_is_502() {
        let login = raas_error(RaasError::LoginRejected { status: 401 });
        assert_eq!(login.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(login.code(), "raas_unavailable");

        let missing = raas_error(RaasError::Http {
            status: 404,
            code: None,
            endpoint: "candidates/x".to_owned(),
        });
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);

        let broken = raas_error(RaasError::Http {
            status: 500,
            code: None,
            endpoint: "jd/list".to_owned(),
        });
        assert_eq!(broken.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(broken.code(), "raas_error");
    }

    #[test]
    fn a_broken_account_fails_the_whole_request_but_a_missing_item_does_not() {
        assert!(item_failure(RaasError::LoginDisabled, "个岗位").is_err());
        let reason = item_failure(
            RaasError::Http {
                status: 404,
                code: None,
                endpoint: "jd/x".to_owned(),
            },
            "个岗位",
        )
        .expect("只算这一条失败");
        assert_eq!(reason, "RAAS 里没有这个岗位");
    }
}
