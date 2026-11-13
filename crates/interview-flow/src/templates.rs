//! 面试配置模板：把一套常用的配置存下来，建面试时直接选。

use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use interview_engine::InterviewConfig;
use interview_platform::auth::AdminUser;
use interview_platform::http::{
    ApiError, ApiResult, ErrorBody, Json, Page, PageQuery, Path, Query,
};
use serde_json::Value;
use sqlx::PgPool;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::Deps;
use crate::model::{CreateTemplate, Template, UpdateTemplate};
use crate::routes::check_config;

pub(crate) fn router() -> OpenApiRouter<Deps> {
    OpenApiRouter::new()
        .routes(routes!(list_templates, create_template))
        .routes(routes!(get_template, update_template, archive_template))
}

#[derive(sqlx::FromRow)]
struct TemplateRow {
    id: Uuid,
    name: String,
    description: Option<String>,
    config: Value,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

const COLUMNS: &str = "id, name, description, config, created_at, updated_at";

impl TemplateRow {
    fn into_template(self) -> ApiResult<Template> {
        Ok(Template {
            id: self.id,
            name: self.name,
            description: self.description,
            config: serde_json::from_value(self.config).map_err(ApiError::internal)?,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

async fn load(db: &PgPool, id: Uuid) -> ApiResult<TemplateRow> {
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM interview_templates WHERE id = $1 AND archived_at IS NULL"
    ))
    .bind(id)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| ApiError::not_found("模板"))
}

/// 模板里的配置。模板不存在时是 `None`。
pub(crate) async fn template_config(db: &PgPool, id: Uuid) -> ApiResult<Option<InterviewConfig>> {
    let config: Option<Value> = sqlx::query_scalar(
        "SELECT config FROM interview_templates WHERE id = $1 AND archived_at IS NULL",
    )
    .bind(id)
    .fetch_optional(db)
    .await?;
    config
        .map(|value| serde_json::from_value(value).map_err(ApiError::internal))
        .transpose()
}

fn check_name(name: &str) -> ApiResult<&str> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiError::invalid_field("name", "模板要有名字"));
    }
    if name.chars().count() > 80 {
        return Err(ApiError::invalid_field("name", "名字最多 80 个字"));
    }
    Ok(name)
}

/// 模板列表
#[utoipa::path(
    get,
    path = "/api/v1/interview-templates",
    tag = "interview-templates",
    params(PageQuery),
    responses((status = 200, description = "一页模板", body = Page<Template>))
)]
async fn list_templates(
    State(deps): State<Deps>,
    _user: AdminUser,
    Query(query): Query<PageQuery>,
) -> ApiResult<Json<Page<Template>>> {
    let db = &deps.ctx.db;
    let pattern = query
        .keyword()
        .map(|k| format!("%{}%", k.replace('%', "\\%").replace('_', "\\_")));
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM interview_templates \
         WHERE archived_at IS NULL AND ($1::text IS NULL OR name ILIKE $1)",
    )
    .bind(&pattern)
    .fetch_one(db)
    .await?;
    let order = query.order_by(
        &[
            ("name", "name"),
            ("created_at", "created_at"),
            ("updated_at", "updated_at"),
        ],
        "updated_at DESC",
    );
    let rows: Vec<TemplateRow> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM interview_templates \
         WHERE archived_at IS NULL AND ($1::text IS NULL OR name ILIKE $1) \
         ORDER BY {order}, id LIMIT $2 OFFSET $3"
    ))
    .bind(&pattern)
    .bind(query.limit())
    .bind(query.offset())
    .fetch_all(db)
    .await?;
    let items = rows
        .into_iter()
        .map(TemplateRow::into_template)
        .collect::<ApiResult<Vec<_>>>()?;
    Ok(Json(Page::new(items, total, &query)))
}

/// 新建模板
#[utoipa::path(
    post,
    path = "/api/v1/interview-templates",
    tag = "interview-templates",
    request_body = CreateTemplate,
    responses(
        (status = 201, description = "建好了", body = Template),
        (status = 422, description = "名字或配置不合要求", body = ErrorBody),
    )
)]
async fn create_template(
    State(deps): State<Deps>,
    user: AdminUser,
    Json(request): Json<CreateTemplate>,
) -> ApiResult<(StatusCode, Json<Template>)> {
    let name = check_name(&request.name)?;
    check_config(&request.config)?;
    let row: TemplateRow = sqlx::query_as(&format!(
        "INSERT INTO interview_templates (id, name, description, config, created_by) \
         VALUES ($1, $2, $3, $4, $5) RETURNING {COLUMNS}"
    ))
    .bind(interview_core::new_id())
    .bind(name)
    .bind(request.description.as_deref().map(str::trim))
    .bind(serde_json::to_value(&request.config).map_err(ApiError::internal)?)
    .bind(user.id)
    .fetch_one(&deps.ctx.db)
    .await?;
    Ok((StatusCode::CREATED, Json(row.into_template()?)))
}

/// 模板详情
#[utoipa::path(
    get,
    path = "/api/v1/interview-templates/{id}",
    tag = "interview-templates",
    params(("id" = Uuid, Path, description = "模板 ID")),
    responses(
        (status = 200, description = "模板", body = Template),
        (status = 404, description = "没有这个模板", body = ErrorBody),
    )
)]
async fn get_template(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Template>> {
    Ok(Json(load(&deps.ctx.db, id).await?.into_template()?))
}

/// 改模板
///
/// 只影响以后用这个模板新建的面试，已经建好的面试不变。
#[utoipa::path(
    patch,
    path = "/api/v1/interview-templates/{id}",
    tag = "interview-templates",
    params(("id" = Uuid, Path, description = "模板 ID")),
    request_body = UpdateTemplate,
    responses(
        (status = 200, description = "改好了", body = Template),
        (status = 422, description = "名字或配置不合要求", body = ErrorBody),
    )
)]
async fn update_template(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateTemplate>,
) -> ApiResult<Json<Template>> {
    let db = &deps.ctx.db;
    let current = load(db, id).await?;
    let name = match &request.name {
        Some(name) => check_name(name)?.to_owned(),
        None => current.name,
    };
    let description = match request.description {
        Some(description) => Some(description.trim().to_owned()).filter(|d| !d.is_empty()),
        None => current.description,
    };
    let config = match request.config {
        Some(config) => {
            check_config(&config)?;
            serde_json::to_value(&config).map_err(ApiError::internal)?
        }
        None => current.config,
    };
    let row: TemplateRow = sqlx::query_as(&format!(
        "UPDATE interview_templates SET name = $2, description = $3, config = $4 \
         WHERE id = $1 RETURNING {COLUMNS}"
    ))
    .bind(id)
    .bind(name)
    .bind(description)
    .bind(config)
    .fetch_one(db)
    .await?;
    Ok(Json(row.into_template()?))
}

/// 归档模板
///
/// 归档后选不到了，用它建过的面试不受影响。
#[utoipa::path(
    delete,
    path = "/api/v1/interview-templates/{id}",
    tag = "interview-templates",
    params(("id" = Uuid, Path, description = "模板 ID")),
    responses(
        (status = 204, description = "归档了"),
        (status = 404, description = "没有这个模板", body = ErrorBody),
    )
)]
async fn archive_template(
    State(deps): State<Deps>,
    _user: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let done = sqlx::query(
        "UPDATE interview_templates SET archived_at = now() \
         WHERE id = $1 AND archived_at IS NULL",
    )
    .bind(id)
    .execute(&deps.ctx.db)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::not_found("模板"));
    }
    Ok(StatusCode::NO_CONTENT)
}
