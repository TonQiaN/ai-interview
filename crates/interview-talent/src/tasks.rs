//! 后台任务：解析简历（`resume.parse`）。
//!
//! 一份简历从上传到能用，要走这几步：取出原件 → PDF 渲染成图片 → 大模型看图转写成全文 →
//! 整理成结构化信息 → 补全候选人的基本信息 → 生成画像。任务可以重复执行：读过的简历不会再读一遍，
//! 只补后面没做完的步骤。

use std::sync::Arc;

use interview_core::{ParseStatus, task_kind};
use interview_llm::ChatModel;
use interview_platform::tasks::{NewTask, Task, TaskError, TaskRegistry, TaskResult};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::deps::Deps;
use crate::document::ResumeContent;
use crate::error::TalentError;
use crate::metering::Metered;
use crate::model::{CandidateProfile, ParsedResume};
use crate::profile::{ProfileInput, generate_profile};
use crate::resume::read_resume;
use crate::store::resumes::ResumeRecord;
use crate::store::{candidates, resumes};

/// `resume.parse` 任务的内容。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ParsePayload {
    pub resume_id: Uuid,
    /// 用简历里的姓名替换候选人现有的名字。上传简历自动建候选人时用，那时的名字是占位的。
    #[serde(default)]
    pub adopt_name: bool,
}

/// 一个「解析这份简历」的任务。同一份简历同时只排一个。
pub(crate) fn parse_task(resume_id: Uuid, adopt_name: bool) -> NewTask {
    NewTask::new(
        task_kind::RESUME_PARSE,
        json!(ParsePayload {
            resume_id,
            adopt_name
        }),
    )
    .dedupe(format!("{}:{resume_id}", task_kind::RESUME_PARSE))
}

/// 登记这条线的后台任务。
pub fn register_tasks(registry: &mut TaskRegistry, deps: Deps) {
    registry.register_fn(task_kind::RESUME_PARSE, move |task| {
        let deps = deps.clone();
        async move { run_parse(&deps, &task).await }
    });
}

/// 给 HR 看的失败原因。
const LLM_MISSING: &str = "还没有配置大模型，简历没法解析。配置好之后点「重新解析」";
const LLM_DOWN: &str = "大模型暂时不可用，简历没有解析成。请稍后点「重新解析」";
const FILE_MISSING: &str = "简历原件找不到了，请重新上传";
const UNFINISHED: &str = "解析中途出错，没有完成。请点「重新解析」";

async fn run_parse(deps: &Deps, task: &Task) -> TaskResult {
    let payload: ParsePayload = task.payload_as()?;
    let last_attempt = task.attempts >= task.max_attempts;
    let result = parse(deps, task, &payload, last_attempt).await;
    // 不会再重试了，就别让简历一直停在「解析中」
    let gave_up = match &result {
        Ok(_) => false,
        Err(TaskError::Fatal(_)) => true,
        Err(TaskError::Retry(_)) => last_attempt,
    };
    if gave_up
        && let Err(error) =
            resumes::fail_if_unfinished(&deps.ctx.db, payload.resume_id, UNFINISHED).await
    {
        tracing::error!(resume_id = %payload.resume_id, %error, "把简历标成解析失败时出错");
    }
    result
}

async fn parse(deps: &Deps, task: &Task, payload: &ParsePayload, last_attempt: bool) -> TaskResult {
    let db = &deps.ctx.db;
    let Some(record) = resumes::get_record(db, payload.resume_id).await? else {
        // 排队的时候简历被删了
        return Ok(Some(json!({ "skipped": "resume_deleted" })));
    };
    let resume = &record.detail.resume;

    // 第一步：读简历。上次已经读完的（任务重试、只是画像没生成）不再读
    let already_parsed = match (&resume.parse_status, &record.detail.parsed) {
        (ParseStatus::Done, Some(parsed)) => Some(parsed.clone()),
        _ => None,
    };
    let parsed = match already_parsed {
        Some(parsed) => parsed,
        None => {
            let Some(llm) = deps.llm.clone() else {
                resumes::mark_failed(db, resume.id, LLM_MISSING).await?;
                return Ok(Some(json!({ "failed": "llm_not_configured" })));
            };
            resumes::mark_running(db, resume.id).await?;
            let model = Metered::new(llm);
            let reading = match load_content(deps, &record).await {
                Ok(content) => read_resume(&model, &content, &deps.ai).await,
                Err(error) => Err(error),
            };
            model
                .flush(db, task_kind::RESUME_PARSE, Some(task.id))
                .await;
            match reading {
                Ok(reading) => {
                    resumes::mark_done(db, resume.id, &reading.text, &reading.parsed).await?;
                    reading.parsed
                }
                Err(error) => return give_up_or_retry(deps, resume.id, error, last_attempt).await,
            }
        }
    };

    // 第二步：主简历的内容用来补全候选人的基本信息和生成画像。
    // 是不是主简历不能看开头读到的那一眼：读简历和写画像各要十几秒，这期间可能有更新的简历
    // 传上来顶替了它。所以每次往候选人身上写的时候再确认，不是主简历了就不写
    let still_primary = candidates::fill_from_resume(
        db,
        resume.candidate_id,
        resume.id,
        &parsed,
        payload.adopt_name,
    )
    .await?;
    if !still_primary {
        return finish(deps, resume.id, false).await;
    }
    let Some(llm) = deps.llm.clone() else {
        return finish(deps, resume.id, false).await;
    };
    let notes = candidates::get(db, resume.candidate_id)
        .await?
        .and_then(|candidate| candidate.notes);
    match build_profile(deps, llm, &parsed, notes.as_deref(), Some(task.id)).await {
        Ok(profile) => {
            let saved =
                candidates::set_profile_from_resume(db, resume.candidate_id, resume.id, &profile)
                    .await?;
            finish(deps, resume.id, saved).await
        }
        Err(TalentError::Llm(error)) if error.is_retryable() && !last_attempt => {
            Err(TaskError::retry(error))
        }
        // 画像生成不出来不算简历解析失败：简历已经能用了，画像可以之后手动重新生成
        Err(error) => {
            tracing::warn!(candidate_id = %resume.candidate_id, %error, "简历解析好了，但画像没有生成出来");
            finish(deps, resume.id, false).await
        }
    }
}

/// 任务收尾。
///
/// 任务跑的过程中如果有人点了「重新解析」，上次的结果已经被清掉，而那次请求排的任务
/// 因为和我们是同一份简历，被并到了我们这一个上。这时不能当作做完了，要再跑一遍，
/// 不然简历会一直停在「排队中」没人管。
async fn finish(deps: &Deps, resume_id: Uuid, profile: bool) -> TaskResult {
    if resumes::is_pending(&deps.ctx.db, resume_id).await? {
        return Err(TaskError::retry(anyhow::anyhow!(
            "解析过程中又被要求重新解析，再跑一遍"
        )));
    }
    Ok(Some(json!({ "parsed": true, "profile": profile })))
}

/// 读简历出错了：是文件的问题就直接标失败，是大模型一时的问题就重试。
async fn give_up_or_retry(
    deps: &Deps,
    resume_id: Uuid,
    error: TalentError,
    last_attempt: bool,
) -> TaskResult {
    let db = &deps.ctx.db;
    match error {
        TalentError::Llm(error) if error.is_retryable() && !last_attempt => {
            Err(TaskError::retry(error))
        }
        TalentError::Llm(error) => {
            resumes::mark_failed(db, resume_id, LLM_DOWN).await?;
            Err(TaskError::fatal(error))
        }
        TalentError::Config(_) => {
            resumes::mark_failed(db, resume_id, UNFINISHED).await?;
            Err(TaskError::fatal(error))
        }
        // 剩下的都是这份文件本身的问题（空的、打不开、不是简历），重试没用，原因告诉 HR
        error => {
            resumes::mark_failed(db, resume_id, &error.to_string()).await?;
            Ok(Some(json!({ "failed": "unreadable" })))
        }
    }
}

/// 取出简历的内容，准备给大模型读。
async fn load_content(deps: &Deps, record: &ResumeRecord) -> Result<ResumeContent, TalentError> {
    let resume = &record.detail.resume;
    match (&record.blob_key, &record.detail.text_content) {
        (Some(key), _) => {
            let bytes = deps.ctx.blobs.get(key).await.map_err(|error| {
                tracing::error!(resume_id = %resume.id, %error, "取简历原件失败");
                TalentError::Render(FILE_MISSING.to_owned())
            })?;
            ResumeContent::load(&bytes, resume.file_name.as_deref(), &deps.renderer).await
        }
        // 没有原件、只有文字的简历（从别的系统导入的）
        (None, Some(text)) if !text.trim().is_empty() => Ok(ResumeContent::Text(text.clone())),
        _ => Err(TalentError::EmptyFile),
    }
}

/// 根据一份解析好的简历生成画像，并把大模型用量记下来。不存库。
pub(crate) async fn build_profile(
    deps: &Deps,
    llm: Arc<dyn ChatModel>,
    parsed: &ParsedResume,
    notes: Option<&str>,
    task_id: Option<Uuid>,
) -> Result<CandidateProfile, TalentError> {
    let model = Metered::new(llm);
    let result = generate_profile(
        &model,
        ProfileInput {
            resume: parsed,
            notes,
        },
        &deps.ai,
    )
    .await;
    model
        .flush(&deps.ctx.db, "candidate.profile", task_id)
        .await;
    result.map(|(profile, _usage)| profile)
}
