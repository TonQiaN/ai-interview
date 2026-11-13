//! `candidates` 表的读写。

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use interview_platform::http::{Page, PageQuery};
use serde::{Deserialize, Serialize};
use sqlx::types::Json;
use sqlx::{PgConnection, PgPool, Postgres, QueryBuilder};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::input::like_pattern;
use crate::model::{CandidateProfile, ParsedResume};
use crate::text_enum::text_enum;

text_enum! {
    /// 候选人是怎么来的（`candidates.source`）。
    pub enum CandidateSource {
        /// HR 在后台手工建的
        Manual => "manual",
        /// 直接上传简历建的
        Upload => "upload",
        /// 从 RAAS 导入的
        Raas => "raas",
    }
}

/// 一位候选人。
#[derive(Clone, PartialEq, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct Candidate {
    pub id: Uuid,
    pub name: String,
    pub email: Option<String>,
    /// 手机号。大陆手机号存 11 位数字，不带 +86
    pub phone: Option<String>,
    /// 现在的职位
    pub current_title: Option<String>,
    /// 现在的公司
    pub current_company: Option<String>,
    /// 所在城市
    pub location: Option<String>,
    /// 候选人画像：大模型根据简历归纳的水平、亮点、疑点和面试要核实的地方。还没生成时各字段是空的
    #[sqlx(json)]
    pub profile: CandidateProfile,
    /// 画像上次生成的时间。没生成过是空
    pub profile_updated_at: Option<DateTime<Utc>>,
    /// HR 写的备注
    pub notes: Option<String>,
    pub source: CandidateSource,
    /// 在来源系统里的编号。手工建的没有
    pub external_ref: Option<String>,
    /// 上次从来源系统同步的时间
    pub external_synced_at: Option<DateTime<Utc>>,
    /// 谁建的
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// 归档时间。归档的候选人不出现在列表里，但已有的面试还能看到
    pub archived_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for Candidate {
    // 候选人的姓名和联系方式不进日志
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Candidate")
            .field("id", &self.id)
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

/// 可能是同一个人的另一位候选人：手机号或邮箱一样。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct DuplicateHint {
    pub id: Uuid,
    pub name: String,
    pub current_title: Option<String>,
    pub current_company: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl std::fmt::Debug for DuplicateHint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DuplicateHint")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// 候选人列表的筛选条件。和分页参数一起用。
#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CandidateFilter {
    /// 只看这个来源的
    pub source: Option<CandidateSource>,
    /// 把归档的也列出来。默认不列
    pub include_archived: Option<bool>,
}

/// 要新建的候选人。字段已经由调用方收拾干净。
#[derive(Clone)]
pub struct NewCandidate {
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub current_title: Option<String>,
    pub current_company: Option<String>,
    pub location: Option<String>,
    pub notes: Option<String>,
    pub source: CandidateSource,
    pub external_ref: Option<String>,
    pub created_by: Option<Uuid>,
}

/// 对一位候选人的修改。`None` 表示这一项不动。
#[derive(Clone, Default)]
pub struct CandidateChanges {
    pub name: Option<String>,
    pub email: Option<Option<String>>,
    pub phone: Option<Option<String>>,
    pub current_title: Option<Option<String>>,
    pub current_company: Option<Option<String>>,
    pub location: Option<Option<String>>,
    pub notes: Option<Option<String>>,
}

/// 修改的结果。
#[derive(Debug, Clone, PartialEq)]
pub enum UpdateOutcome {
    Updated(Box<Candidate>),
    NotFound,
    /// 已经归档，不能再改
    Archived,
}

const COLUMNS: &str = "id, name, email, phone, current_title, current_company, location, profile, \
                       profile_updated_at, notes, source, external_ref, external_synced_at, \
                       created_by, created_at, updated_at, archived_at";

/// 列表可以按这些字段排序：参数名 → 列名。
const SORTABLE: &[(&str, &str)] = &[
    ("created_at", "created_at"),
    ("updated_at", "updated_at"),
    ("name", "name"),
];

/// 新建一位候选人。可以传事务里的连接，和简历一起建。
pub async fn insert(conn: &mut PgConnection, new: NewCandidate) -> sqlx::Result<Candidate> {
    sqlx::query_as(&format!(
        "INSERT INTO candidates (id, name, email, phone, current_title, current_company, location,
                                 notes, source, external_ref, external_synced_at, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                 CASE WHEN $10::text IS NULL THEN NULL ELSE now() END, $11)
         RETURNING {COLUMNS}"
    ))
    .bind(interview_core::new_id())
    .bind(new.name)
    .bind(new.email)
    .bind(new.phone)
    .bind(new.current_title)
    .bind(new.current_company)
    .bind(new.location)
    .bind(new.notes)
    .bind(new.source)
    .bind(new.external_ref)
    .bind(new.created_by)
    .fetch_one(conn)
    .await
}

/// 按来源系统里的编号取一位候选人。归档的也能取到。
pub async fn get_by_external_ref(
    db: &PgPool,
    source: CandidateSource,
    external_ref: &str,
) -> sqlx::Result<Option<Candidate>> {
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM candidates WHERE source = $1 AND external_ref = $2"
    ))
    .bind(source)
    .bind(external_ref)
    .fetch_optional(db)
    .await
}

/// 这些来源编号里，哪些已经导入过（不含归档的）：来源编号 → 我们的候选人 id。
pub async fn imported_ids(
    db: &PgPool,
    source: CandidateSource,
    external_refs: &[String],
) -> sqlx::Result<HashMap<String, Uuid>> {
    if external_refs.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(String, Uuid)> = sqlx::query_as(
        "SELECT external_ref, id FROM candidates
         WHERE source = $1 AND external_ref = ANY($2) AND archived_at IS NULL",
    )
    .bind(source)
    .bind(external_refs)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().collect())
}

/// 用来源系统里的信息补全一位导入的候选人：已经有值的不动（HR 可能改过），归档的顺便恢复。
pub async fn fill_from_source(
    conn: &mut PgConnection,
    id: Uuid,
    new: &NewCandidate,
) -> sqlx::Result<Candidate> {
    sqlx::query_as(&format!(
        "UPDATE candidates
         SET email = COALESCE(email, $2),
             phone = COALESCE(phone, $3),
             current_title = COALESCE(current_title, $4),
             current_company = COALESCE(current_company, $5),
             location = COALESCE(location, $6),
             external_synced_at = now(),
             archived_at = NULL
         WHERE id = $1
         RETURNING {COLUMNS}"
    ))
    .bind(id)
    .bind(&new.email)
    .bind(&new.phone)
    .bind(&new.current_title)
    .bind(&new.current_company)
    .bind(&new.location)
    .fetch_one(conn)
    .await
}

/// 按 id 取一位候选人。归档的也能取到。
pub async fn get(db: &PgPool, id: Uuid) -> sqlx::Result<Option<Candidate>> {
    sqlx::query_as(&format!("SELECT {COLUMNS} FROM candidates WHERE id = $1"))
        .bind(id)
        .fetch_optional(db)
        .await
}

/// 分页列出候选人。关键词在姓名、手机、邮箱、现在的职位和公司里找。
pub async fn list(
    db: &PgPool,
    page: &PageQuery,
    filter: &CandidateFilter,
) -> sqlx::Result<Page<Candidate>> {
    let mut count = QueryBuilder::new("SELECT count(*) FROM candidates");
    push_filters(&mut count, page, filter);
    let total: i64 = count.build_query_scalar().fetch_one(db).await?;

    let mut select = QueryBuilder::new(format!("SELECT {COLUMNS} FROM candidates"));
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

fn push_filters(
    query: &mut QueryBuilder<'_, Postgres>,
    page: &PageQuery,
    filter: &CandidateFilter,
) {
    query.push(" WHERE true");
    if !filter.include_archived.unwrap_or(false) {
        query.push(" AND archived_at IS NULL");
    }
    if let Some(source) = filter.source {
        query.push(" AND source = ").push_bind(source);
    }
    if let Some(keyword) = page.keyword() {
        let pattern = like_pattern(keyword);
        query.push(" AND (");
        for (index, column) in ["name", "phone", "email", "current_title", "current_company"]
            .into_iter()
            .enumerate()
        {
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push(column)
                .push(" ILIKE ")
                .push_bind(pattern.clone());
        }
        query.push(")");
    }
}

/// 修改一位候选人。先锁住这一行再改。
pub async fn update(
    db: &PgPool,
    id: Uuid,
    changes: CandidateChanges,
) -> sqlx::Result<UpdateOutcome> {
    let mut tx = db.begin().await?;
    let current: Option<Candidate> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM candidates WHERE id = $1 FOR UPDATE"
    ))
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(mut candidate) = current else {
        return Ok(UpdateOutcome::NotFound);
    };
    if candidate.archived_at.is_some() {
        return Ok(UpdateOutcome::Archived);
    }
    if let Some(name) = changes.name {
        candidate.name = name;
    }
    if let Some(email) = changes.email {
        candidate.email = email;
    }
    if let Some(phone) = changes.phone {
        candidate.phone = phone;
    }
    if let Some(current_title) = changes.current_title {
        candidate.current_title = current_title;
    }
    if let Some(current_company) = changes.current_company {
        candidate.current_company = current_company;
    }
    if let Some(location) = changes.location {
        candidate.location = location;
    }
    if let Some(notes) = changes.notes {
        candidate.notes = notes;
    }
    let updated = sqlx::query_as(&format!(
        "UPDATE candidates
         SET name = $2, email = $3, phone = $4, current_title = $5, current_company = $6,
             location = $7, notes = $8
         WHERE id = $1
         RETURNING {COLUMNS}"
    ))
    .bind(candidate.id)
    .bind(&candidate.name)
    .bind(&candidate.email)
    .bind(&candidate.phone)
    .bind(&candidate.current_title)
    .bind(&candidate.current_company)
    .bind(&candidate.location)
    .bind(&candidate.notes)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(UpdateOutcome::Updated(Box::new(updated)))
}

/// 归档一位候选人。返回 `false` 表示候选人不存在；已经归档的再归档一次也算成功。
pub async fn archive(db: &PgPool, id: Uuid) -> sqlx::Result<bool> {
    let done = sqlx::query(
        "UPDATE candidates SET archived_at = COALESCE(archived_at, now()) WHERE id = $1",
    )
    .bind(id)
    .execute(db)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// 找手机号或邮箱一样的其他候选人（不含归档的），最多 5 位。两样都没有就不找。
pub async fn find_duplicates(
    db: &PgPool,
    exclude: Option<Uuid>,
    email: Option<&str>,
    phone: Option<&str>,
) -> sqlx::Result<Vec<DuplicateHint>> {
    if email.is_none() && phone.is_none() {
        return Ok(Vec::new());
    }
    sqlx::query_as(
        "SELECT id, name, current_title, current_company, created_at
         FROM candidates
         WHERE archived_at IS NULL
           AND ($1::uuid IS NULL OR id <> $1)
           AND ((email IS NOT NULL AND lower(email) = lower($2)) OR (phone IS NOT NULL AND phone = $3))
         ORDER BY created_at DESC
         LIMIT 5",
    )
    .bind(exclude)
    .bind(email)
    .bind(phone)
    .fetch_all(db)
    .await
}

/// 用简历里读出来的信息补全候选人的基本信息：已经有值的不动。
///
/// `placeholder_name` 是候选人现在的占位名（上传简历自动建的候选人，名字先用文件名顶着）。
/// 给了的话，写的那一刻名字还是它才换成简历里的姓名：读简历要十几秒，这期间 HR 可能已经
/// 自己把名字填上了，填过的不动。
///
/// 只有这份简历现在还是主简历时才写。读简历的这段时间里可能有更新的简历传上来顶替了它，
/// 所以在写的那一刻再确认一次。返回有没有写。
pub async fn fill_from_resume(
    db: &PgPool,
    id: Uuid,
    resume_id: Uuid,
    resume: &ParsedResume,
    placeholder_name: Option<&str>,
) -> sqlx::Result<bool> {
    let done = sqlx::query(
        "UPDATE candidates
         SET name = CASE WHEN name = $2 AND $3::text IS NOT NULL THEN $3 ELSE name END,
             email = COALESCE(email, $4),
             phone = COALESCE(phone, $5),
             current_title = COALESCE(current_title, $6),
             current_company = COALESCE(current_company, $7),
             location = COALESCE(location, $8)
         WHERE id = $1 AND archived_at IS NULL
           AND EXISTS (SELECT 1 FROM resumes
                       WHERE resumes.id = $9 AND resumes.candidate_id = $1 AND resumes.is_primary)",
    )
    .bind(id)
    .bind(placeholder_name)
    .bind(&resume.name)
    .bind(&resume.email)
    .bind(&resume.phone)
    .bind(&resume.current_title)
    .bind(&resume.current_company)
    .bind(&resume.location)
    .bind(resume_id)
    .execute(db)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// 存一份根据某份简历生成的画像。只有这份简历现在还是主简历时才存，返回有没有存。
pub async fn set_profile_from_resume(
    db: &PgPool,
    id: Uuid,
    resume_id: Uuid,
    profile: &CandidateProfile,
) -> sqlx::Result<bool> {
    let done = sqlx::query(
        "UPDATE candidates SET profile = $2, profile_updated_at = now()
         WHERE id = $1
           AND EXISTS (SELECT 1 FROM resumes
                       WHERE resumes.id = $3 AND resumes.candidate_id = $1 AND resumes.is_primary)",
    )
    .bind(id)
    .bind(Json(profile))
    .bind(resume_id)
    .execute(db)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// 存画像。
pub async fn set_profile(db: &PgPool, id: Uuid, profile: &CandidateProfile) -> sqlx::Result<()> {
    sqlx::query("UPDATE candidates SET profile = $2, profile_updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(Json(profile))
        .execute(db)
        .await?;
    Ok(())
}
