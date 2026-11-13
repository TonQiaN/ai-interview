//! `resumes` 表的读写。

use chrono::{DateTime, Utc};
use interview_core::ParseStatus;
use serde::{Deserialize, Serialize};
use sqlx::types::Json;
use sqlx::{PgConnection, PgPool};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::model::ParsedResume;

/// 一份简历的基本信息，不含正文。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct Resume {
    pub id: Uuid,
    pub candidate_id: Uuid,
    /// 上传时的文件名
    pub file_name: Option<String>,
    /// 文件类型，比如 `application/pdf`
    pub content_type: Option<String>,
    /// 文件大小，字节
    pub size_bytes: Option<i64>,
    /// 有没有原件可以下载（从别的系统导入的可能只有文字）
    pub has_file: bool,
    /// 解析进度：`pending` 排队中，`running` 正在解析，`done` 好了，`failed` 失败
    pub parse_status: ParseStatus,
    /// 解析失败的原因，给 HR 看的
    pub parse_error: Option<String>,
    pub parsed_at: Option<DateTime<Utc>>,
    /// 是不是这位候选人的主简历。出题和生成画像默认用主简历
    pub is_primary: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 一份简历，带正文和结构化结果。
#[derive(Clone, PartialEq, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct ResumeDetail {
    #[serde(flatten)]
    #[sqlx(flatten)]
    pub resume: Resume,
    /// 简历全文（Markdown）。解析完成后才有
    pub text_content: Option<String>,
    /// 给面试出题和写报告用的简历文字：不带姓名、手机、邮箱、住址。解析完成后才有
    pub interview_text: Option<String>,
    /// 结构化的简历。解析完成后才有
    #[sqlx(json(nullable))]
    pub parsed: Option<ParsedResume>,
}

impl std::fmt::Debug for ResumeDetail {
    // 简历正文不进日志
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResumeDetail")
            .field("resume", &self.resume)
            .finish_non_exhaustive()
    }
}

/// 后台任务要用的一行：比 [`ResumeDetail`] 多一个文件键。
#[derive(Clone, sqlx::FromRow)]
pub struct ResumeRecord {
    #[sqlx(flatten)]
    pub detail: ResumeDetail,
    pub blob_key: Option<String>,
}

/// 要新建的简历。
#[derive(Debug, Clone)]
pub struct NewResume {
    pub id: Uuid,
    pub candidate_id: Uuid,
    pub blob_key: Option<String>,
    pub file_name: Option<String>,
    pub content_type: Option<String>,
    pub size_bytes: Option<i64>,
    pub sha256: Option<String>,
    /// 没有原件、只有文字的简历（从别的系统导入的）把文字放这里
    pub text_content: Option<String>,
}

const COLUMNS: &str = "id, candidate_id, file_name, content_type, size_bytes, \
                       (blob_key IS NOT NULL) AS has_file, parse_status, parse_error, parsed_at, \
                       is_primary, created_at, updated_at";

/// 新建一份简历，并把它设成主简历（原来的主简历让位）。
pub async fn insert_primary(conn: &mut PgConnection, new: NewResume) -> sqlx::Result<Resume> {
    sqlx::query("UPDATE resumes SET is_primary = false WHERE candidate_id = $1 AND is_primary")
        .bind(new.candidate_id)
        .execute(&mut *conn)
        .await?;
    sqlx::query_as(&format!(
        "INSERT INTO resumes (id, candidate_id, blob_key, file_name, content_type, size_bytes,
                              sha256, text_content, is_primary)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, true)
         RETURNING {COLUMNS}"
    ))
    .bind(new.id)
    .bind(new.candidate_id)
    .bind(new.blob_key)
    .bind(new.file_name)
    .bind(new.content_type)
    .bind(new.size_bytes)
    .bind(new.sha256)
    .bind(new.text_content)
    .fetch_one(conn)
    .await
}

pub async fn get(db: &PgPool, id: Uuid) -> sqlx::Result<Option<Resume>> {
    sqlx::query_as(&format!("SELECT {COLUMNS} FROM resumes WHERE id = $1"))
        .bind(id)
        .fetch_optional(db)
        .await
}

pub async fn get_detail(db: &PgPool, id: Uuid) -> sqlx::Result<Option<ResumeDetail>> {
    sqlx::query_as(&format!(
        "SELECT {COLUMNS}, text_content, interview_text, parsed FROM resumes WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(db)
    .await
}

pub async fn get_record(db: &PgPool, id: Uuid) -> sqlx::Result<Option<ResumeRecord>> {
    sqlx::query_as(&format!(
        "SELECT {COLUMNS}, text_content, interview_text, parsed, blob_key FROM resumes WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(db)
    .await
}

/// 一位候选人的全部简历，主简历在前，其余按上传时间倒序。
pub async fn list_for_candidate(db: &PgPool, candidate_id: Uuid) -> sqlx::Result<Vec<Resume>> {
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM resumes WHERE candidate_id = $1
         ORDER BY is_primary DESC, created_at DESC, id DESC"
    ))
    .bind(candidate_id)
    .fetch_all(db)
    .await
}

/// 一批候选人各自的主简历。
pub async fn primary_for(db: &PgPool, candidate_ids: &[Uuid]) -> sqlx::Result<Vec<Resume>> {
    if candidate_ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM resumes WHERE candidate_id = ANY($1) AND is_primary"
    ))
    .bind(candidate_ids)
    .fetch_all(db)
    .await
}

/// 一位候选人的主简历，带正文。
pub async fn primary_detail(db: &PgPool, candidate_id: Uuid) -> sqlx::Result<Option<ResumeDetail>> {
    sqlx::query_as(&format!(
        "SELECT {COLUMNS}, text_content, interview_text, parsed FROM resumes
         WHERE candidate_id = $1 AND is_primary"
    ))
    .bind(candidate_id)
    .fetch_optional(db)
    .await
}

/// 这位候选人是不是已经传过内容一模一样的文件。
pub async fn find_by_sha256(
    db: &PgPool,
    candidate_id: Uuid,
    sha256: &str,
) -> sqlx::Result<Option<Resume>> {
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM resumes WHERE candidate_id = $1 AND sha256 = $2
         ORDER BY created_at DESC LIMIT 1"
    ))
    .bind(candidate_id)
    .bind(sha256)
    .fetch_optional(db)
    .await
}

/// 开始解析。
pub async fn mark_running(db: &PgPool, id: Uuid) -> sqlx::Result<()> {
    sqlx::query("UPDATE resumes SET parse_status = 'running', parse_error = NULL WHERE id = $1")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

/// 解析好了：存全文和结构化结果。
pub async fn mark_done(
    db: &PgPool,
    id: Uuid,
    text: &str,
    parsed: &ParsedResume,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE resumes
         SET parse_status = 'done', parse_error = NULL, parsed_at = now(),
             text_content = $2, parsed = $3, interview_text = $4
         WHERE id = $1",
    )
    .bind(id)
    .bind(text)
    .bind(Json(parsed))
    .bind(parsed.to_interview_text())
    .execute(db)
    .await?;
    Ok(())
}

/// 解析失败。`reason` 会显示给 HR。
pub async fn mark_failed(db: &PgPool, id: Uuid, reason: &str) -> sqlx::Result<()> {
    sqlx::query("UPDATE resumes SET parse_status = 'failed', parse_error = $2 WHERE id = $1")
        .bind(id)
        .bind(reason)
        .execute(db)
        .await?;
    Ok(())
}

/// 解析没跑完就停了（任务用完了重试次数）：还停在「解析中」的改成失败。
pub async fn fail_if_unfinished(db: &PgPool, id: Uuid, reason: &str) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE resumes SET parse_status = 'failed', parse_error = $2
         WHERE id = $1 AND parse_status IN ('pending', 'running')",
    )
    .bind(id)
    .bind(reason)
    .execute(db)
    .await?;
    Ok(())
}

/// 准备重新解析：状态回到排队中，丢掉上次的结果。返回 `false` 表示简历不存在。
pub async fn reset_for_parse(conn: &mut PgConnection, id: Uuid) -> sqlx::Result<bool> {
    let done = sqlx::query(
        "UPDATE resumes
         SET parse_status = 'pending', parse_error = NULL, parsed = NULL, parsed_at = NULL,
             interview_text = NULL
         WHERE id = $1",
    )
    .bind(id)
    .execute(conn)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// 删掉一份简历，返回它的文件键（调用方接着删文件）。删的是主简历时，最新的一份顶上。
/// 返回 `None` 表示简历不存在。
pub async fn delete(db: &PgPool, id: Uuid) -> sqlx::Result<Option<Option<String>>> {
    let mut tx = db.begin().await?;
    let deleted: Option<(Uuid, Option<String>, bool)> = sqlx::query_as(
        "DELETE FROM resumes WHERE id = $1 RETURNING candidate_id, blob_key, is_primary",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((candidate_id, blob_key, was_primary)) = deleted else {
        return Ok(None);
    };
    if was_primary {
        sqlx::query(
            "UPDATE resumes SET is_primary = true
             WHERE id = (SELECT id FROM resumes WHERE candidate_id = $1
                         ORDER BY created_at DESC, id DESC LIMIT 1)",
        )
        .bind(candidate_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Some(blob_key))
}
