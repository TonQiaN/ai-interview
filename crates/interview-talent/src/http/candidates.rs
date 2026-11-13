//! 候选人接口：增删改查、上传简历自动建候选人、重新生成画像。

use std::collections::HashMap;

use axum::extract::multipart::MultipartRejection;
use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::http::StatusCode;
use interview_platform::auth::AdminUser;
use interview_platform::http::{ErrorBody, Json, Page, PageQuery, Path, Query};
use interview_platform::{ApiError, ApiResult};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use super::talent_error;
use super::upload::{self, MAX_UPLOAD_BODY_BYTES, ResumeUploadForm};
use crate::deps::Deps;
use crate::input::{self, LONG_TEXT_MAX, SHORT_TEXT_MAX};
use crate::resume::normalize_phone;
use crate::store::candidates::{
    self, Candidate, CandidateChanges, CandidateFilter, CandidateSource, DuplicateHint,
    NewCandidate, UpdateOutcome,
};
use crate::store::resumes::{self, Resume};
use crate::tasks::build_profile;

pub(super) fn router() -> OpenApiRouter<Deps> {
    let uploads = OpenApiRouter::new()
        .routes(routes!(import_candidate))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BODY_BYTES));
    OpenApiRouter::new()
        .routes(routes!(list_candidates, create_candidate))
        .routes(routes!(get_candidate, update_candidate, archive_candidate))
        .routes(routes!(regenerate_profile))
        .merge(uploads)
}

/// 列表里的一位候选人。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CandidateListItem {
    #[serde(flatten)]
    pub candidate: Candidate,
    /// 主简历。看它的 `parse_status` 知道简历是不是还在解析
    pub primary_resume: Option<Resume>,
}

/// 一位候选人的详情。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CandidateDetail {
    #[serde(flatten)]
    pub candidate: Candidate,
    /// 主简历。看它的 `parse_status` 知道简历是不是还在解析
    pub primary_resume: Option<Resume>,
    /// 手机号或邮箱一样的其他候选人，可能是同一个人
    pub possible_duplicates: Vec<DuplicateHint>,
}

/// 新建候选人的请求。
#[derive(Deserialize, ToSchema)]
pub struct CreateCandidate {
    /// 姓名，必填
    pub name: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub current_title: Option<String>,
    #[serde(default)]
    pub current_company: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    /// 手机号或邮箱和已有的候选人一样时，默认返回 409；确认不是同一个人就传 `true` 照样建
    #[serde(default)]
    pub allow_duplicate: bool,
}

/// 修改候选人的请求。只写要改的字段；可以为空的字段写 `null` 表示清空。
#[derive(Default, Deserialize, ToSchema)]
pub struct UpdateCandidate {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "input::nullable")]
    #[schema(value_type = Option<String>)]
    pub email: Option<Option<String>>,
    #[serde(default, deserialize_with = "input::nullable")]
    #[schema(value_type = Option<String>)]
    pub phone: Option<Option<String>>,
    #[serde(default, deserialize_with = "input::nullable")]
    #[schema(value_type = Option<String>)]
    pub current_title: Option<Option<String>>,
    #[serde(default, deserialize_with = "input::nullable")]
    #[schema(value_type = Option<String>)]
    pub current_company: Option<Option<String>>,
    #[serde(default, deserialize_with = "input::nullable")]
    #[schema(value_type = Option<String>)]
    pub location: Option<Option<String>>,
    #[serde(default, deserialize_with = "input::nullable")]
    #[schema(value_type = Option<String>)]
    pub notes: Option<Option<String>>,
}

/// 上传简历自动建候选人的结果。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImportedCandidate {
    /// 新建的候选人。名字先用文件名占位，简历解析完会换成简历里的姓名
    pub candidate: Candidate,
    /// 刚上传的简历，正在排队解析
    pub resume: Resume,
}

/// 邮箱：去空白、转小写，粗查一下格式。
fn clean_email(value: Option<String>) -> ApiResult<Option<String>> {
    let Some(email) = input::optional("email", value, SHORT_TEXT_MAX)? else {
        return Ok(None);
    };
    let email = email.to_lowercase();
    let looks_right = email
        .split_once('@')
        .is_some_and(|(user, domain)| !user.is_empty() && domain.contains('.'))
        && !email.contains(char::is_whitespace);
    if !looks_right {
        return Err(ApiError::invalid_field("email", "邮箱格式不对"));
    }
    Ok(Some(email))
}

/// 手机号：统一成一种写法，认不出来的报错。
fn clean_phone(value: Option<String>) -> ApiResult<Option<String>> {
    let Some(phone) = input::optional("phone", value, SHORT_TEXT_MAX)? else {
        return Ok(None);
    };
    normalize_phone(&phone)
        .map(Some)
        .ok_or_else(|| ApiError::invalid_field("phone", "手机号格式不对"))
}

/// 候选人加上主简历和可能的重复。
async fn detail(deps: &Deps, candidate: Candidate) -> ApiResult<CandidateDetail> {
    let db = &deps.ctx.db;
    let primary_resume = resumes::primary_for(db, &[candidate.id])
        .await?
        .into_iter()
        .next();
    let possible_duplicates = if candidate.archived_at.is_some() {
        Vec::new()
    } else {
        candidates::find_duplicates(
            db,
            Some(candidate.id),
            candidate.email.as_deref(),
            candidate.phone.as_deref(),
        )
        .await?
    };
    Ok(CandidateDetail {
        candidate,
        primary_resume,
        possible_duplicates,
    })
}

/// 候选人列表
///
/// 关键词 `q` 在姓名、手机、邮箱、现在的职位和公司里找。`sort` 可以是 `created_at`、
/// `updated_at`、`name`，默认 `-created_at`。
#[utoipa::path(
    get,
    path = "/api/v1/candidates",
    tag = "candidates",
    params(PageQuery, CandidateFilter),
    responses((status = 200, description = "一页候选人", body = Page<CandidateListItem>))
)]
async fn list_candidates(
    State(deps): State<Deps>,
    _user: AdminUser,
    Query(page): Query<PageQuery>,
    Query(filter): Query<CandidateFilter>,
) -> ApiResult<Json<Page<CandidateListItem>>> {
    let db = &deps.ctx.db;
    let found = candidates::list(db, &page, &filter).await?;
    let ids: Vec<Uuid> = found.items.iter().map(|candidate| candidate.id).collect();
    let mut primary: HashMap<Uuid, Resume> = resumes::primary_for(db, &ids)
        .await?
        .into_iter()
        .map(|resume| (resume.candidate_id, resume))
        .collect();
    Ok(Json(found.map(|candidate| CandidateListItem {
        primary_resume: primary.remove(&candidate.id),
        candidate,
    })))
}

/// 新建候选人
///
/// 手工填基本信息。有简历的话更省事的做法是直接调「上传简历建候选人」。
#[utoipa::path(
    post,
    path = "/api/v1/candidates",
    tag = "candidates",
    request_body = CreateCandidate,
    responses(
        (status = 201, description = "建好的候选人", body = CandidateDetail),
        (status = 409, description = "手机号或邮箱和已有的候选人一样，code 是 duplicate_candidate，details.duplicates 里是那些候选人", body = ErrorBody),
        (status = 422, description = "字段不合要求", body = ErrorBody),
    )
)]
async fn create_candidate(
    State(deps): State<Deps>,
    user: AdminUser,
    Json(request): Json<CreateCandidate>,
) -> ApiResult<(StatusCode, Json<CandidateDetail>)> {
    let db = &deps.ctx.db;
    let new = NewCandidate {
        name: input::required("name", &request.name, SHORT_TEXT_MAX)?,
        email: clean_email(request.email)?,
        phone: clean_phone(request.phone)?,
        current_title: input::optional("current_title", request.current_title, SHORT_TEXT_MAX)?,
        current_company: input::optional(
            "current_company",
            request.current_company,
            SHORT_TEXT_MAX,
        )?,
        location: input::optional("location", request.location, SHORT_TEXT_MAX)?,
        notes: input::optional("notes", request.notes, LONG_TEXT_MAX)?,
        source: CandidateSource::Manual,
        external_ref: None,
        created_by: Some(user.id),
    };
    if !request.allow_duplicate {
        let duplicates =
            candidates::find_duplicates(db, None, new.email.as_deref(), new.phone.as_deref())
                .await?;
        if !duplicates.is_empty() {
            return Err(
                ApiError::conflict("已经有手机号或邮箱一样的候选人，可能是同一个人")
                    .with_code("duplicate_candidate")
                    .with_details(json!({ "duplicates": duplicates })),
            );
        }
    }
    let mut conn = db.acquire().await?;
    let candidate = candidates::insert(&mut conn, new).await?;
    drop(conn);
    Ok((StatusCode::CREATED, Json(detail(&deps, candidate).await?)))
}

/// 上传简历建候选人
///
/// 直接传一份简历（`multipart/form-data`，字段 `file`；或者字段 `text` 传粘贴的文字），
/// 自动建一位候选人。立刻返回 202，简历在后台解析：读出姓名、联系方式、经历，生成画像。
/// 前端轮询候选人详情，看 `primary_resume.parse_status`。解析完如果发现和已有的候选人
/// 手机号或邮箱一样，详情的 `possible_duplicates` 里会列出来。
#[utoipa::path(
    post,
    path = "/api/v1/candidates/import",
    tag = "candidates",
    request_body(content = ResumeUploadForm, content_type = "multipart/form-data"),
    responses(
        (status = 202, description = "候选人已建好，简历在排队解析", body = ImportedCandidate),
        (status = 413, description = "文件超过 20 MB", body = ErrorBody),
        (status = 422, description = "没传文件，或文件类型不支持（code 是 unsupported_file、empty_file）", body = ErrorBody),
    )
)]
async fn import_candidate(
    State(deps): State<Deps>,
    user: AdminUser,
    multipart: Result<Multipart, MultipartRejection>,
) -> ApiResult<(StatusCode, Json<ImportedCandidate>)> {
    let upload = upload::read_upload(multipart).await?;
    let mut tx = deps.ctx.db.begin().await?;
    let candidate = candidates::insert(
        &mut tx,
        NewCandidate {
            name: upload::placeholder_name(upload.file_name.as_deref()),
            email: None,
            phone: None,
            current_title: None,
            current_company: None,
            location: None,
            notes: None,
            source: CandidateSource::Upload,
            external_ref: None,
            created_by: Some(user.id),
        },
    )
    .await?;
    let resume = upload::store_resume(&deps, &mut tx, candidate.id, upload, true).await?;
    tx.commit().await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(ImportedCandidate { candidate, resume }),
    ))
}

/// 候选人详情
///
/// 归档的候选人也能取到（已有的面试还要显示），看 `archived_at`。
#[utoipa::path(
    get,
    path = "/api/v1/candidates/{id}",
    tag = "candidates",
    params(("id" = Uuid, Path, description = "候选人 id")),
    responses(
        (status = 200, description = "候选人", body = CandidateDetail),
        (status = 404, description = "候选人不存在", body = ErrorBody),
    )
)]
async fn get_candidate(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<CandidateDetail>> {
    let candidate = candidates::get(&deps.ctx.db, id)
        .await?
        .ok_or_else(|| ApiError::not_found("候选人"))?;
    Ok(Json(detail(&deps, candidate).await?))
}

/// 修改候选人
#[utoipa::path(
    patch,
    path = "/api/v1/candidates/{id}",
    tag = "candidates",
    params(("id" = Uuid, Path, description = "候选人 id")),
    request_body = UpdateCandidate,
    responses(
        (status = 200, description = "改好的候选人", body = CandidateDetail),
        (status = 404, description = "候选人不存在", body = ErrorBody),
        (status = 409, description = "候选人已归档", body = ErrorBody),
        (status = 422, description = "字段不合要求", body = ErrorBody),
    )
)]
async fn update_candidate(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateCandidate>,
) -> ApiResult<Json<CandidateDetail>> {
    let text = |field: &str,
                value: Option<Option<String>>,
                max: usize|
     -> ApiResult<Option<Option<String>>> {
        value
            .map(|value| input::optional(field, value, max))
            .transpose()
    };
    let changes = CandidateChanges {
        name: request
            .name
            .map(|name| input::required("name", &name, SHORT_TEXT_MAX))
            .transpose()?,
        email: request.email.map(clean_email).transpose()?,
        phone: request.phone.map(clean_phone).transpose()?,
        current_title: text("current_title", request.current_title, SHORT_TEXT_MAX)?,
        current_company: text("current_company", request.current_company, SHORT_TEXT_MAX)?,
        location: text("location", request.location, SHORT_TEXT_MAX)?,
        notes: text("notes", request.notes, LONG_TEXT_MAX)?,
    };
    match candidates::update(&deps.ctx.db, id, changes).await? {
        UpdateOutcome::Updated(candidate) => Ok(Json(detail(&deps, *candidate).await?)),
        UpdateOutcome::NotFound => Err(ApiError::not_found("候选人")),
        UpdateOutcome::Archived => Err(ApiError::conflict("候选人已归档，不能再修改")),
    }
}

/// 归档候选人
///
/// 归档后不出现在列表里，数据还在，已有的面试不受影响。
#[utoipa::path(
    delete,
    path = "/api/v1/candidates/{id}",
    tag = "candidates",
    params(("id" = Uuid, Path, description = "候选人 id")),
    responses(
        (status = 204, description = "已归档"),
        (status = 404, description = "候选人不存在", body = ErrorBody),
    )
)]
async fn archive_candidate(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if candidates::archive(&deps.ctx.db, id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found("候选人"))
    }
}

/// 重新生成画像
///
/// 根据主简历和备注让大模型重写画像。改了备注、换了简历之后用。一般要等十秒左右，
/// 完成后直接返回新的候选人详情。
#[utoipa::path(
    post,
    path = "/api/v1/candidates/{id}/profile/regenerate",
    tag = "candidates",
    params(("id" = Uuid, Path, description = "候选人 id")),
    responses(
        (status = 200, description = "带新画像的候选人", body = CandidateDetail),
        (status = 404, description = "候选人不存在", body = ErrorBody),
        (status = 409, description = "还没有解析好的简历，code 是 resume_not_parsed", body = ErrorBody),
        (status = 503, description = "大模型没配置或暂时不可用，code 是 llm_unavailable", body = ErrorBody),
    )
)]
async fn regenerate_profile(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<CandidateDetail>> {
    let db = &deps.ctx.db;
    let candidate = candidates::get(db, id)
        .await?
        .ok_or_else(|| ApiError::not_found("候选人"))?;
    if candidate.archived_at.is_some() {
        return Err(ApiError::conflict("候选人已归档，不能再修改"));
    }
    let parsed = resumes::primary_detail(db, id)
        .await?
        .and_then(|resume| resume.parsed)
        .ok_or_else(|| {
            ApiError::conflict("还没有解析好的简历，没法生成画像").with_code("resume_not_parsed")
        })?;
    let llm = deps.require_llm()?;
    let profile = build_profile(&deps, llm, &parsed, candidate.notes.as_deref(), None)
        .await
        .map_err(talent_error)?;
    candidates::set_profile(db, id, &profile).await?;
    let candidate = candidates::get(db, id)
        .await?
        .ok_or_else(|| ApiError::not_found("候选人"))?;
    Ok(Json(detail(&deps, candidate).await?))
}
