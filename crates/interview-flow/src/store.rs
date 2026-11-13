//! 数据库读写。只碰 ① 归属的表；岗位、候选人、简历、音色只读。

use chrono::{DateTime, Utc};
use interview_core::{InterviewDecision, InterviewStatus, ReportStatus, SynthStatus};
use interview_engine::{InterviewConfig, InterviewPlan, Utterance, UtteranceKind};
use interview_platform::http::PageQuery;
use serde_json::Value;
use sqlx::{PgConnection, PgPool, Postgres, QueryBuilder};
use uuid::Uuid;

use crate::model::{
    CandidateBrief, InterviewDetail, InterviewFilter, InterviewSummary, PositionBrief, ReportBrief,
    SynthProgress, UtteranceView,
};

/// `interviews` 表的一行。
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct InterviewRow {
    pub id: Uuid,
    pub candidate_id: Uuid,
    pub position_id: Uuid,
    pub resume_id: Option<Uuid>,
    pub template_id: Option<Uuid>,
    pub voice_id: Option<Uuid>,
    pub status: InterviewStatus,
    pub config: Value,
    pub plan: Option<Value>,
    pub prepare_error: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub end_reason: Option<String>,
    pub decision: InterviewDecision,
    pub decision_note: Option<String>,
    pub decided_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const INTERVIEW_COLUMNS: &str = "id, candidate_id, position_id, resume_id, template_id, voice_id, \
     status, config, plan, prepare_error, started_at, ended_at, end_reason, decision, \
     decision_note, decided_at, created_at, updated_at";

impl InterviewRow {
    /// 配置。库里的 JSON 缺字段时用默认值补上。
    pub fn config(&self) -> Result<InterviewConfig, serde_json::Error> {
        serde_json::from_value(self.config.clone())
    }

    /// 题目。还没出题时是 `None`。
    pub fn plan(&self) -> Result<Option<InterviewPlan>, serde_json::Error> {
        self.plan.clone().map(serde_json::from_value).transpose()
    }
}

pub(crate) async fn get_interview(
    db: &PgPool,
    id: Uuid,
) -> Result<Option<InterviewRow>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {INTERVIEW_COLUMNS} FROM interviews WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(db)
    .await
}

fn push_filters(
    builder: &mut QueryBuilder<'_, Postgres>,
    filter: &InterviewFilter,
    query: &PageQuery,
) {
    if !filter.include_archived.unwrap_or(false) {
        builder.push(" AND i.archived_at IS NULL");
    }
    if let Some(candidate_id) = filter.candidate_id {
        builder
            .push(" AND i.candidate_id = ")
            .push_bind(candidate_id);
    }
    if let Some(position_id) = filter.position_id {
        builder.push(" AND i.position_id = ").push_bind(position_id);
    }
    if let Some(status) = filter.status {
        builder.push(" AND i.status = ").push_bind(status);
    }
    if let Some(keyword) = query.keyword() {
        let pattern = format!("%{}%", keyword.replace('%', "\\%").replace('_', "\\_"));
        builder
            .push(" AND (c.name ILIKE ")
            .push_bind(pattern.clone())
            .push(" OR p.title ILIKE ")
            .push_bind(pattern)
            .push(")");
    }
}

const LIST_FROM: &str = " FROM interviews i \
     JOIN candidates c ON c.id = i.candidate_id \
     JOIN positions p ON p.id = i.position_id \
     WHERE true";

pub(crate) async fn list_interviews(
    db: &PgPool,
    filter: &InterviewFilter,
    query: &PageQuery,
) -> Result<(Vec<InterviewSummary>, i64), sqlx::Error> {
    let mut count = QueryBuilder::new(format!("SELECT count(*){LIST_FROM}"));
    push_filters(&mut count, filter, query);
    let total: i64 = count.build_query_scalar().fetch_one(db).await?;

    let order = query.order_by(
        &[
            ("created_at", "i.created_at"),
            ("updated_at", "i.updated_at"),
            ("started_at", "i.started_at"),
            ("candidate", "c.name"),
            ("position", "p.title"),
            ("status", "i.status"),
        ],
        "i.created_at DESC",
    );
    let mut list = QueryBuilder::new(format!(
        "SELECT i.id, i.status, i.decision, i.candidate_id, c.name AS candidate_name, \
         i.position_id, p.title AS position_title, \
         (SELECT r.status FROM interview_reports r WHERE r.interview_id = i.id \
          ORDER BY r.version DESC LIMIT 1) AS report_status, \
         i.started_at, i.ended_at, i.created_at, i.updated_at{LIST_FROM}"
    ));
    push_filters(&mut list, filter, query);
    list.push(format!(" ORDER BY {order}, i.id LIMIT "))
        .push_bind(query.limit())
        .push(" OFFSET ")
        .push_bind(query.offset());
    let items = list.build_query_as().fetch_all(db).await?;
    Ok((items, total))
}

pub(crate) async fn synth_progress(db: &PgPool, id: Uuid) -> Result<SynthProgress, sqlx::Error> {
    sqlx::query_as(
        "SELECT count(*) AS total, \
                count(*) FILTER (WHERE synth_status = 'done') AS done, \
                count(*) FILTER (WHERE synth_status = 'failed') AS failed \
         FROM interview_utterances WHERE interview_id = $1 AND source = 'planned'",
    )
    .bind(id)
    .fetch_one(db)
    .await
}

/// 把一行面试拼成详情：带上候选人和岗位的摘要、合成进度、最新的报告。
pub(crate) async fn detail(
    db: &PgPool,
    row: InterviewRow,
) -> Result<InterviewDetail, anyhow::Error> {
    let (name, current_title, current_company): (String, Option<String>, Option<String>) =
        sqlx::query_as("SELECT name, current_title, current_company FROM candidates WHERE id = $1")
            .bind(row.candidate_id)
            .fetch_one(db)
            .await?;
    let (title, department): (String, Option<String>) =
        sqlx::query_as("SELECT title, department FROM positions WHERE id = $1")
            .bind(row.position_id)
            .fetch_one(db)
            .await?;
    let report: Option<(Uuid, i32, ReportStatus, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT id, version, status, generated_at FROM interview_reports \
         WHERE interview_id = $1 ORDER BY version DESC LIMIT 1",
    )
    .bind(row.id)
    .fetch_optional(db)
    .await?;
    let synth = synth_progress(db, row.id).await?;
    let config = row.config()?;

    Ok(InterviewDetail {
        id: row.id,
        status: row.status,
        candidate: CandidateBrief {
            id: row.candidate_id,
            name,
            current_title,
            current_company,
        },
        position: PositionBrief {
            id: row.position_id,
            title,
            department,
        },
        resume_id: row.resume_id,
        template_id: row.template_id,
        voice_id: row.voice_id,
        time_budget: config.time_budget(),
        config,
        has_plan: row.plan.is_some(),
        synth,
        prepare_error: row.prepare_error,
        started_at: row.started_at,
        ended_at: row.ended_at,
        end_reason: row.end_reason,
        decision: row.decision,
        decision_note: row.decision_note,
        decided_at: row.decided_at,
        report: report.map(|(id, version, status, generated_at)| ReportBrief {
            id,
            version,
            status,
            generated_at,
        }),
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// 某张表里有没有这一行（没归档的）。表名只能是代码里写死的。
pub(crate) async fn exists(
    db: &PgPool,
    table: &'static str,
    id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(&format!(
        "SELECT EXISTS (SELECT 1 FROM {table} WHERE id = $1 AND archived_at IS NULL)"
    ))
    .bind(id)
    .fetch_one(db)
    .await
}

/// 这份简历是不是这个候选人的。
pub(crate) async fn resume_belongs_to(
    db: &PgPool,
    resume_id: Uuid,
    candidate_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM resumes WHERE id = $1 AND candidate_id = $2)")
        .bind(resume_id)
        .bind(candidate_id)
        .fetch_one(db)
        .await
}

pub(crate) struct NewInterview<'a> {
    pub candidate_id: Uuid,
    pub position_id: Uuid,
    pub resume_id: Option<Uuid>,
    pub template_id: Option<Uuid>,
    pub voice_id: Option<Uuid>,
    pub config: &'a InterviewConfig,
    pub created_by: Uuid,
}

pub(crate) async fn insert_interview(
    db: &PgPool,
    new: NewInterview<'_>,
) -> Result<InterviewRow, anyhow::Error> {
    let row = sqlx::query_as(&format!(
        "INSERT INTO interviews \
         (id, candidate_id, position_id, resume_id, template_id, voice_id, config, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING {INTERVIEW_COLUMNS}"
    ))
    .bind(interview_core::new_id())
    .bind(new.candidate_id)
    .bind(new.position_id)
    .bind(new.resume_id)
    .bind(new.template_id)
    .bind(new.voice_id)
    .bind(serde_json::to_value(new.config)?)
    .bind(new.created_by)
    .fetch_one(db)
    .await?;
    Ok(row)
}

/// 带条件地改状态：只有当前状态在 `from` 里才改。返回改没改成。
pub(crate) async fn set_status(
    conn: &mut PgConnection,
    id: Uuid,
    from: &[InterviewStatus],
    to: InterviewStatus,
    prepare_error: Option<&str>,
) -> Result<bool, sqlx::Error> {
    let from: Vec<&str> = from.iter().map(|s| s.as_str()).collect();
    let done = sqlx::query(
        "UPDATE interviews SET status = $3, prepare_error = $4 \
         WHERE id = $1 AND status = ANY($2)",
    )
    .bind(id)
    .bind(from)
    .bind(to)
    .bind(prepare_error)
    .execute(conn)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// 把计划里的话同步到 `interview_utterances`。
///
/// 文字没变的保留原来的音频；变了的清掉音频、等重新合成；计划里没有了的删掉。
/// `resynth_all` 为真时全部重新合成（换了音色）。
pub(crate) async fn sync_utterances(
    conn: &mut PgConnection,
    interview_id: Uuid,
    utterances: &[Utterance],
    resynth_all: bool,
) -> Result<(), sqlx::Error> {
    let keys: Vec<&str> = utterances.iter().map(|u| u.key.as_str()).collect();
    sqlx::query(
        "DELETE FROM interview_utterances \
         WHERE interview_id = $1 AND source = 'planned' AND key <> ALL($2)",
    )
    .bind(interview_id)
    .bind(&keys)
    .execute(&mut *conn)
    .await?;

    for (seq, utterance) in utterances.iter().enumerate() {
        sqlx::query(
            "INSERT INTO interview_utterances (id, interview_id, key, seq, kind, text, source) \
             VALUES ($1, $2, $3, $4, $5, $6, 'planned') \
             ON CONFLICT (interview_id, key) DO UPDATE SET \
                 seq = EXCLUDED.seq, \
                 kind = EXCLUDED.kind, \
                 synth_status = CASE WHEN interview_utterances.text = EXCLUDED.text AND NOT $7 \
                                     THEN interview_utterances.synth_status ELSE 'pending' END, \
                 synth_error = CASE WHEN interview_utterances.text = EXCLUDED.text AND NOT $7 \
                                    THEN interview_utterances.synth_error ELSE NULL END, \
                 text = EXCLUDED.text",
        )
        .bind(interview_core::new_id())
        .bind(interview_id)
        .bind(&utterance.key)
        .bind(i32::try_from(seq).unwrap_or(i32::MAX))
        .bind(kind_text(utterance.kind))
        .bind(&utterance.text)
        .bind(resynth_all)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

fn kind_text(kind: UtteranceKind) -> &'static str {
    match kind {
        UtteranceKind::Opening => "opening",
        UtteranceKind::Question => "question",
        UtteranceKind::FollowUp => "follow_up",
        UtteranceKind::Transition => "transition",
        UtteranceKind::Closing => "closing",
        UtteranceKind::Other => "other",
    }
}

pub(crate) async fn list_utterances(
    db: &PgPool,
    interview_id: Uuid,
) -> Result<Vec<UtteranceView>, sqlx::Error> {
    sqlx::query_as(
        "SELECT key, seq, kind, text, source, synth_status, synth_error, tts_engine, \
                audio_duration_ms, \
                (audio_blob_key IS NOT NULL AND synth_status = 'done') AS has_audio \
         FROM interview_utterances WHERE interview_id = $1 ORDER BY seq, key",
    )
    .bind(interview_id)
    .fetch_all(db)
    .await
}

/// 还没合成好的话：`(key, text)`。
pub(crate) async fn pending_utterances(
    db: &PgPool,
    interview_id: Uuid,
) -> Result<Vec<(String, String)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT key, text FROM interview_utterances \
         WHERE interview_id = $1 AND source = 'planned' AND synth_status <> 'done' \
         ORDER BY seq, key",
    )
    .bind(interview_id)
    .fetch_all(db)
    .await
}

pub(crate) struct SynthResult<'a> {
    pub key: &'a str,
    pub status: SynthStatus,
    pub engine: &'a str,
    pub blob_key: Option<&'a str>,
    pub duration_ms: Option<i32>,
    pub error: Option<&'a str>,
}

pub(crate) async fn save_synth_result(
    db: &PgPool,
    interview_id: Uuid,
    result: SynthResult<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE interview_utterances SET synth_status = $3, tts_engine = $4, \
             audio_blob_key = $5, audio_content_type = CASE WHEN $5 IS NULL THEN NULL ELSE 'audio/wav' END, \
             audio_duration_ms = $6, synth_error = $7 \
         WHERE interview_id = $1 AND key = $2",
    )
    .bind(interview_id)
    .bind(result.key)
    .bind(result.status)
    .bind(result.engine)
    .bind(result.blob_key)
    .bind(result.duration_ms)
    .bind(result.error)
    .execute(db)
    .await?;
    Ok(())
}

/// 一句话的音频在哪：`(blob_key, content_type)`。
pub(crate) async fn utterance_audio(
    db: &PgPool,
    interview_id: Uuid,
    key: &str,
) -> Result<Option<(String, Option<String>)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT audio_blob_key, audio_content_type FROM interview_utterances \
         WHERE interview_id = $1 AND key = $2 AND audio_blob_key IS NOT NULL \
               AND synth_status = 'done'",
    )
    .bind(interview_id)
    .bind(key)
    .fetch_optional(db)
    .await
}
