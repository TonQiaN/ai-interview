//! 出题和写报告时要读的岗位和简历。
//!
//! 各列的读法见 `docs/architecture/data-model.md` 的「出题和写报告时怎么读岗位和简历」。

use sqlx::PgPool;
use uuid::Uuid;

use crate::scrub::scrub_contacts;

/// 给大模型看的岗位。
pub(crate) struct JobText {
    pub title: String,
    /// 公司或团队的展示名，没有就是空的
    pub company: String,
    /// 岗位描述加任职要求
    pub description: String,
}

pub(crate) async fn job(db: &PgPool, position_id: Uuid) -> Result<JobText, sqlx::Error> {
    let (title, company, description, requirements): (String, Option<String>, String, String) =
        sqlx::query_as(
            "SELECT title, extra->>'client_name', description, requirements \
             FROM positions WHERE id = $1",
        )
        .bind(position_id)
        .fetch_one(db)
        .await?;
    let mut text = description.trim().to_owned();
    if !requirements.trim().is_empty() {
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str("任职要求：\n");
        text.push_str(requirements.trim());
    }
    Ok(JobText {
        title,
        company: company.unwrap_or_default(),
        description: text,
    })
}

/// 给大模型看的简历。
pub(crate) struct ResumeText {
    /// 去掉了姓名和联系方式的简历文字。没有简历时是空的
    pub text: String,
    /// 简历还在解析，过一会儿才有文字
    pub parsing: bool,
}

/// 读这场面试用的简历：指定了就用指定的，没指定用候选人的主简历。
pub(crate) async fn resume(
    db: &PgPool,
    candidate_id: Uuid,
    resume_id: Option<Uuid>,
) -> Result<ResumeText, sqlx::Error> {
    // interview_text 是去掉了姓名和联系方式的简历；老数据没有这一列时退回全文
    const RESUME: &str = "SELECT COALESCE(interview_text, text_content), parse_status FROM resumes";
    let row: Option<(Option<String>, String)> =
        match resume_id {
            Some(resume_id) => {
                sqlx::query_as(&format!("{RESUME} WHERE id = $1"))
                    .bind(resume_id)
                    .fetch_optional(db)
                    .await?
            }
            None => sqlx::query_as(&format!(
                "{RESUME} WHERE candidate_id = $1 ORDER BY is_primary DESC, created_at DESC LIMIT 1"
            ))
            .bind(candidate_id)
            .fetch_optional(db)
            .await?,
        };
    Ok(match row {
        Some((text, status)) => ResumeText {
            // 电话和邮箱大模型用不上。interview_text 里本来就没有，这里再挡一道
            text: scrub_contacts(&text.unwrap_or_default()),
            parsing: status == "pending" || status == "running",
        },
        None => ResumeText {
            text: String::new(),
            parsing: false,
        },
    })
}
