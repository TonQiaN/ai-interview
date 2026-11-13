//! `positions` 表的读写。

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use interview_platform::http::{Page, PageQuery};
use serde::{Deserialize, Serialize};
use sqlx::types::Json;
use sqlx::{PgConnection, PgPool, Postgres, QueryBuilder};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::input::like_pattern;
use crate::model::PositionExtra;
use crate::text_enum::text_enum;

text_enum! {
    /// 岗位状态（`positions.status`）。
    pub enum PositionStatus {
        /// 在招
        Open => "open",
        /// 暂停
        Paused => "paused",
        /// 已关闭
        Closed => "closed",
    }
}

text_enum! {
    /// 岗位是怎么来的（`positions.source`）。
    pub enum PositionSource {
        /// HR 在后台手工建的
        Manual => "manual",
        /// 从 RAAS 导入的
        Raas => "raas",
    }
}

/// 一个岗位。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct Position {
    pub id: Uuid,
    /// 岗位名称
    pub title: String,
    /// 部门
    pub department: Option<String>,
    /// 工作地点
    pub location: Option<String>,
    /// 用工类型，比如「全职」「实习」「外包」
    pub employment_type: Option<String>,
    /// 级别，比如「高级」「P6」
    pub seniority: Option<String>,
    /// 岗位描述全文（做什么）。出题时原样交给大模型
    pub description: String,
    /// 任职要求全文（要什么人）。出题时原样交给大模型
    pub requirements: String,
    /// 结构化的补充信息：必备技能、加分项、薪资、面试考察重点等
    #[sqlx(json)]
    pub extra: PositionExtra,
    pub source: PositionSource,
    /// 在来源系统里的编号。手工建的没有
    pub external_ref: Option<String>,
    /// 上次从来源系统同步的时间
    pub external_synced_at: Option<DateTime<Utc>>,
    pub status: PositionStatus,
    /// 谁建的
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// 归档时间。归档的岗位不出现在列表里，但已有的面试还能看到它
    pub archived_at: Option<DateTime<Utc>>,
}

/// 岗位列表的筛选条件。和分页参数一起用。
#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PositionFilter {
    /// 只看这个状态的
    pub status: Option<PositionStatus>,
    /// 只看这个来源的
    pub source: Option<PositionSource>,
    /// 把归档的也列出来。默认不列
    pub include_archived: Option<bool>,
}

/// 要新建的岗位。字段已经由调用方收拾干净。
#[derive(Debug, Clone)]
pub struct NewPosition {
    pub title: String,
    pub department: Option<String>,
    pub location: Option<String>,
    pub employment_type: Option<String>,
    pub seniority: Option<String>,
    pub description: String,
    pub requirements: String,
    pub extra: PositionExtra,
    pub status: PositionStatus,
    pub source: PositionSource,
    pub external_ref: Option<String>,
    pub created_by: Option<Uuid>,
}

/// 对一个岗位的修改。`None` 表示这一项不动。
#[derive(Debug, Clone, Default)]
pub struct PositionChanges {
    pub title: Option<String>,
    pub department: Option<Option<String>>,
    pub location: Option<Option<String>>,
    pub employment_type: Option<Option<String>>,
    pub seniority: Option<Option<String>>,
    pub description: Option<String>,
    pub requirements: Option<String>,
    pub extra: Option<PositionExtra>,
    pub status: Option<PositionStatus>,
}

/// 修改的结果。
#[derive(Debug, Clone, PartialEq)]
pub enum UpdateOutcome {
    Updated(Box<Position>),
    NotFound,
    /// 已经归档，不能再改
    Archived,
}

const COLUMNS: &str = "id, title, department, location, employment_type, seniority, description, \
                       requirements, extra, source, external_ref, external_synced_at, status, \
                       created_by, created_at, updated_at, archived_at";

/// 列表可以按这些字段排序：参数名 → 列名。
const SORTABLE: &[(&str, &str)] = &[
    ("created_at", "created_at"),
    ("updated_at", "updated_at"),
    ("title", "title"),
    ("status", "status"),
];

pub async fn insert(db: &PgPool, new: NewPosition) -> sqlx::Result<Position> {
    sqlx::query_as(&format!(
        "INSERT INTO positions (id, title, department, location, employment_type, seniority,
                                description, requirements, extra, status, source, external_ref,
                                external_synced_at, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
                 CASE WHEN $12::text IS NULL THEN NULL ELSE now() END, $13)
         RETURNING {COLUMNS}"
    ))
    .bind(interview_core::new_id())
    .bind(new.title)
    .bind(new.department)
    .bind(new.location)
    .bind(new.employment_type)
    .bind(new.seniority)
    .bind(new.description)
    .bind(new.requirements)
    .bind(Json(new.extra))
    .bind(new.status)
    .bind(new.source)
    .bind(new.external_ref)
    .bind(new.created_by)
    .fetch_one(db)
    .await
}

/// 按来源系统里的编号取一个岗位。归档的也能取到。
pub async fn get_by_external_ref(
    db: &PgPool,
    source: PositionSource,
    external_ref: &str,
) -> sqlx::Result<Option<Position>> {
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM positions WHERE source = $1 AND external_ref = $2"
    ))
    .bind(source)
    .bind(external_ref)
    .fetch_optional(db)
    .await
}

/// 这些来源编号里，哪些已经导入过（不含归档的）：来源编号 → 我们的岗位 id。
pub async fn imported_ids(
    db: &PgPool,
    source: PositionSource,
    external_refs: &[String],
) -> sqlx::Result<HashMap<String, Uuid>> {
    if external_refs.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(String, Uuid)> = sqlx::query_as(
        "SELECT external_ref, id FROM positions
         WHERE source = $1 AND external_ref = ANY($2) AND archived_at IS NULL",
    )
    .bind(source)
    .bind(external_refs)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().collect())
}

/// 用来源系统里最新的内容覆盖一个导入的岗位，归档的顺便恢复。
pub async fn overwrite_from_source(
    db: &PgPool,
    id: Uuid,
    new: NewPosition,
) -> sqlx::Result<Position> {
    sqlx::query_as(&format!(
        "UPDATE positions
         SET title = $2, department = $3, location = $4, employment_type = $5, seniority = $6,
             description = $7, requirements = $8, extra = $9, status = $10,
             external_synced_at = now(), archived_at = NULL
         WHERE id = $1
         RETURNING {COLUMNS}"
    ))
    .bind(id)
    .bind(new.title)
    .bind(new.department)
    .bind(new.location)
    .bind(new.employment_type)
    .bind(new.seniority)
    .bind(new.description)
    .bind(new.requirements)
    .bind(Json(new.extra))
    .bind(new.status)
    .fetch_one(db)
    .await
}

/// 按 id 取一个岗位。归档的也能取到。
pub async fn get(db: &PgPool, id: Uuid) -> sqlx::Result<Option<Position>> {
    sqlx::query_as(&format!("SELECT {COLUMNS} FROM positions WHERE id = $1"))
        .bind(id)
        .fetch_optional(db)
        .await
}

/// 分页列出岗位。关键词在名称、部门、地点、客户名里找。
pub async fn list(
    db: &PgPool,
    page: &PageQuery,
    filter: &PositionFilter,
) -> sqlx::Result<Page<Position>> {
    let mut count = QueryBuilder::new("SELECT count(*) FROM positions");
    push_filters(&mut count, page, filter);
    let total: i64 = count.build_query_scalar().fetch_one(db).await?;

    let mut select = QueryBuilder::new(format!("SELECT {COLUMNS} FROM positions"));
    push_filters(&mut select, page, filter);
    // order_by 的结果只会是白名单里的列名，可以直接拼
    select
        .push(" ORDER BY ")
        .push(page.order_by(SORTABLE, "created_at DESC"))
        .push(", id DESC LIMIT ")
        .push_bind(page.limit())
        .push(" OFFSET ")
        .push_bind(page.offset());
    let items = select.build_query_as().fetch_all(db).await?;

    Ok(Page::new(items, total, page))
}

fn push_filters(query: &mut QueryBuilder<'_, Postgres>, page: &PageQuery, filter: &PositionFilter) {
    query.push(" WHERE true");
    if !filter.include_archived.unwrap_or(false) {
        query.push(" AND archived_at IS NULL");
    }
    if let Some(status) = filter.status {
        query.push(" AND status = ").push_bind(status);
    }
    if let Some(source) = filter.source {
        query.push(" AND source = ").push_bind(source);
    }
    if let Some(keyword) = page.keyword() {
        let pattern = like_pattern(keyword);
        query
            .push(" AND (title ILIKE ")
            .push_bind(pattern.clone())
            .push(" OR department ILIKE ")
            .push_bind(pattern.clone())
            .push(" OR location ILIKE ")
            .push_bind(pattern.clone())
            .push(" OR extra->>'client_name' ILIKE ")
            .push_bind(pattern)
            .push(")");
    }
}

/// 修改一个岗位。先锁住这一行再改，两个人同时改不同字段不会互相覆盖。
pub async fn update(
    db: &PgPool,
    id: Uuid,
    changes: PositionChanges,
) -> sqlx::Result<UpdateOutcome> {
    let mut tx = db.begin().await?;
    let current: Option<Position> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM positions WHERE id = $1 FOR UPDATE"
    ))
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(mut position) = current else {
        return Ok(UpdateOutcome::NotFound);
    };
    if position.archived_at.is_some() {
        return Ok(UpdateOutcome::Archived);
    }
    apply(&mut position, changes);
    let updated = write(&mut tx, &position).await?;
    tx.commit().await?;
    Ok(UpdateOutcome::Updated(Box::new(updated)))
}

fn apply(position: &mut Position, changes: PositionChanges) {
    if let Some(title) = changes.title {
        position.title = title;
    }
    if let Some(department) = changes.department {
        position.department = department;
    }
    if let Some(location) = changes.location {
        position.location = location;
    }
    if let Some(employment_type) = changes.employment_type {
        position.employment_type = employment_type;
    }
    if let Some(seniority) = changes.seniority {
        position.seniority = seniority;
    }
    if let Some(description) = changes.description {
        position.description = description;
    }
    if let Some(requirements) = changes.requirements {
        position.requirements = requirements;
    }
    if let Some(mut extra) = changes.extra {
        // 来源系统里的编号不归 HR 改，保留原来的
        extra.raas = position.extra.raas.take();
        position.extra = extra;
    }
    if let Some(status) = changes.status {
        position.status = status;
    }
}

async fn write(conn: &mut PgConnection, position: &Position) -> sqlx::Result<Position> {
    sqlx::query_as(&format!(
        "UPDATE positions
         SET title = $2, department = $3, location = $4, employment_type = $5, seniority = $6,
             description = $7, requirements = $8, extra = $9, status = $10
         WHERE id = $1
         RETURNING {COLUMNS}"
    ))
    .bind(position.id)
    .bind(&position.title)
    .bind(&position.department)
    .bind(&position.location)
    .bind(&position.employment_type)
    .bind(&position.seniority)
    .bind(&position.description)
    .bind(&position.requirements)
    .bind(Json(&position.extra))
    .bind(position.status)
    .fetch_one(conn)
    .await
}

/// 归档一个岗位。返回 `false` 表示岗位不存在；已经归档的再归档一次也算成功。
pub async fn archive(db: &PgPool, id: Uuid) -> sqlx::Result<bool> {
    let done = sqlx::query(
        "UPDATE positions SET archived_at = COALESCE(archived_at, now()) WHERE id = $1",
    )
    .bind(id)
    .execute(db)
    .await?;
    Ok(done.rows_affected() == 1)
}
