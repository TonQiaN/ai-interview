//! 岗位接口：增删改查，以及把一段岗位描述整理成结构化草稿。

use axum::extract::State;
use axum::http::StatusCode;
use interview_platform::auth::AdminUser;
use interview_platform::http::{ErrorBody, Json, Page, PageQuery, Path, Query};
use interview_platform::{ApiError, ApiResult};
use serde::Deserialize;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use super::talent_error;
use crate::deps::Deps;
use crate::input::{self, LONG_TEXT_MAX, SHORT_TEXT_MAX};
use crate::metering::Metered;
use crate::model::PositionExtra;
use crate::position::{PositionDraft, structure_position};
use crate::store::positions::{
    self, NewPosition, Position, PositionChanges, PositionFilter, PositionSource, PositionStatus,
    UpdateOutcome,
};

pub(super) fn router() -> OpenApiRouter<Deps> {
    OpenApiRouter::new()
        .routes(routes!(list_positions, create_position))
        .routes(routes!(draft_position))
        .routes(routes!(get_position, update_position, archive_position))
}

/// 新建岗位的请求。结构和 [`PositionDraft`] 一样，整理出的草稿确认后可以原样提交。
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePosition {
    /// 岗位名称，必填
    pub title: String,
    #[serde(default)]
    pub department: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub employment_type: Option<String>,
    #[serde(default)]
    pub seniority: Option<String>,
    /// 岗位描述全文
    #[serde(default)]
    pub description: String,
    /// 任职要求全文
    #[serde(default)]
    pub requirements: String,
    #[serde(default)]
    pub extra: PositionExtra,
    /// 不填就是在招（`open`）
    #[serde(default)]
    pub status: Option<PositionStatus>,
}

/// 修改岗位的请求。只写要改的字段；可以为空的字段写 `null` 表示清空。
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct UpdatePosition {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, deserialize_with = "input::nullable")]
    #[schema(value_type = Option<String>)]
    pub department: Option<Option<String>>,
    #[serde(default, deserialize_with = "input::nullable")]
    #[schema(value_type = Option<String>)]
    pub location: Option<Option<String>>,
    #[serde(default, deserialize_with = "input::nullable")]
    #[schema(value_type = Option<String>)]
    pub employment_type: Option<Option<String>>,
    #[serde(default, deserialize_with = "input::nullable")]
    #[schema(value_type = Option<String>)]
    pub seniority: Option<Option<String>>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub requirements: Option<String>,
    /// 整个替换，不是合并
    #[serde(default)]
    pub extra: Option<PositionExtra>,
    #[serde(default)]
    pub status: Option<PositionStatus>,
}

/// 整理岗位描述的请求。
#[derive(Debug, Deserialize, ToSchema)]
pub struct DraftPositionRequest {
    /// 岗位描述的原文，从招聘网站、文档或聊天记录里粘贴过来的都行
    pub text: String,
}

/// 岗位列表
///
/// 关键词 `q` 在名称、部门、地点、客户名里找。`sort` 可以是 `created_at`、`updated_at`、
/// `title`、`status`，默认 `-created_at`。
#[utoipa::path(
    get,
    path = "/api/v1/positions",
    tag = "positions",
    params(PageQuery, PositionFilter),
    responses((status = 200, description = "一页岗位", body = Page<Position>))
)]
async fn list_positions(
    State(deps): State<Deps>,
    _user: AdminUser,
    Query(page): Query<PageQuery>,
    Query(filter): Query<PositionFilter>,
) -> ApiResult<Json<Page<Position>>> {
    Ok(Json(positions::list(&deps.ctx.db, &page, &filter).await?))
}

/// 新建岗位
#[utoipa::path(
    post,
    path = "/api/v1/positions",
    tag = "positions",
    request_body = CreatePosition,
    responses(
        (status = 201, description = "建好的岗位", body = Position),
        (status = 422, description = "字段不合要求", body = ErrorBody),
    )
)]
async fn create_position(
    State(deps): State<Deps>,
    user: AdminUser,
    Json(request): Json<CreatePosition>,
) -> ApiResult<(StatusCode, Json<Position>)> {
    let mut extra = request.extra;
    // 来源系统里的编号只在导入时写
    extra.raas = None;
    let new = NewPosition {
        title: input::required("title", &request.title, SHORT_TEXT_MAX)?,
        department: input::optional("department", request.department, SHORT_TEXT_MAX)?,
        location: input::optional("location", request.location, SHORT_TEXT_MAX)?,
        employment_type: input::optional(
            "employment_type",
            request.employment_type,
            SHORT_TEXT_MAX,
        )?,
        seniority: input::optional("seniority", request.seniority, SHORT_TEXT_MAX)?,
        description: input::body("description", &request.description, LONG_TEXT_MAX)?,
        requirements: input::body("requirements", &request.requirements, LONG_TEXT_MAX)?,
        extra,
        status: request.status.unwrap_or(PositionStatus::Open),
        source: PositionSource::Manual,
        external_ref: None,
        created_by: Some(user.id),
    };
    let position = positions::insert(&deps.ctx.db, new).await?;
    Ok((StatusCode::CREATED, Json(position)))
}

/// 把一段岗位描述整理成草稿
///
/// 让大模型读粘贴进来的原文，分出名称、部门、描述、任职要求、必备技能、考察重点等。
/// 只返回草稿，不保存；HR 确认或修改后再调新建接口。一般要等几秒。
#[utoipa::path(
    post,
    path = "/api/v1/positions/draft",
    tag = "positions",
    request_body = DraftPositionRequest,
    responses(
        (status = 200, description = "整理好的草稿", body = PositionDraft),
        (status = 422, description = "内容不是岗位描述，code 是 not_a_position", body = ErrorBody),
        (status = 503, description = "大模型没配置或暂时不可用，code 是 llm_unavailable", body = ErrorBody),
    )
)]
async fn draft_position(
    State(deps): State<Deps>,
    _user: AdminUser,
    Json(request): Json<DraftPositionRequest>,
) -> ApiResult<Json<PositionDraft>> {
    let model = Metered::new(deps.require_llm()?);
    let result = structure_position(&model, &request.text, &deps.ai).await;
    // 失败的那几次调用也花了钱，一样要记
    model.flush(&deps.ctx.db, "position.structure", None).await;
    let (draft, _usage) = result.map_err(talent_error)?;
    Ok(Json(draft))
}

/// 岗位详情
///
/// 归档的岗位也能取到（已有的面试还要显示它），看 `archived_at`。
#[utoipa::path(
    get,
    path = "/api/v1/positions/{id}",
    tag = "positions",
    params(("id" = Uuid, Path, description = "岗位 id")),
    responses(
        (status = 200, description = "岗位", body = Position),
        (status = 404, description = "岗位不存在", body = ErrorBody),
    )
)]
async fn get_position(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Position>> {
    positions::get(&deps.ctx.db, id)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("岗位"))
}

/// 修改岗位
#[utoipa::path(
    patch,
    path = "/api/v1/positions/{id}",
    tag = "positions",
    params(("id" = Uuid, Path, description = "岗位 id")),
    request_body = UpdatePosition,
    responses(
        (status = 200, description = "改好的岗位", body = Position),
        (status = 404, description = "岗位不存在", body = ErrorBody),
        (status = 409, description = "岗位已归档", body = ErrorBody),
        (status = 422, description = "字段不合要求", body = ErrorBody),
    )
)]
async fn update_position(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdatePosition>,
) -> ApiResult<Json<Position>> {
    let short = |field: &str, value: Option<Option<String>>| -> ApiResult<Option<Option<String>>> {
        value
            .map(|value| input::optional(field, value, SHORT_TEXT_MAX))
            .transpose()
    };
    let changes = PositionChanges {
        title: request
            .title
            .map(|title| input::required("title", &title, SHORT_TEXT_MAX))
            .transpose()?,
        department: short("department", request.department)?,
        location: short("location", request.location)?,
        employment_type: short("employment_type", request.employment_type)?,
        seniority: short("seniority", request.seniority)?,
        description: request
            .description
            .map(|text| input::body("description", &text, LONG_TEXT_MAX))
            .transpose()?,
        requirements: request
            .requirements
            .map(|text| input::body("requirements", &text, LONG_TEXT_MAX))
            .transpose()?,
        extra: request.extra,
        status: request.status,
    };
    match positions::update(&deps.ctx.db, id, changes).await? {
        UpdateOutcome::Updated(position) => Ok(Json(*position)),
        UpdateOutcome::NotFound => Err(ApiError::not_found("岗位")),
        UpdateOutcome::Archived => Err(ApiError::conflict("岗位已归档，不能再修改")),
    }
}

/// 归档岗位
///
/// 归档后不出现在列表里，数据还在，已有的面试不受影响。
#[utoipa::path(
    delete,
    path = "/api/v1/positions/{id}",
    tag = "positions",
    params(("id" = Uuid, Path, description = "岗位 id")),
    responses(
        (status = 204, description = "已归档"),
        (status = 404, description = "岗位不存在", body = ErrorBody),
    )
)]
async fn archive_position(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if positions::archive(&deps.ctx.db, id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found("岗位"))
    }
}
