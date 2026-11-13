//! 逐题面试的进行：开始、推进、取面试官语音、结束。
//!
//! 下一步说什么由 `interview_engine::room` 决定。这里做的是它周围的事：存状态
//! （`interviews.runtime_state`）、记对话（`interview_turns`）、给每句话补上语音地址、
//! 在后台判断候选人的回答、面试结束时把转写和出报告交出去（`interview.finalize` 任务）。
//!
//! 几条规矩：
//!
//! - 状态的每次改动都在锁住这场面试那一行的事务里做，重复的请求不会把面试推进两次。
//! - 判断回答要调模型，可能要几秒，不能占着事务。先不拿锁算出结果，写的时候再确认
//!   状态没被别人动过；同一个进程里另有一把锁，保证同一段回答只判断一次。
//! - 候选人的回答是个人信息：只进数据库，不进日志。

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use chrono::{DateTime, Utc};
use interview_core::status::{InterviewStatus, UtteranceKind};
use interview_core::task_kind;
use interview_engine::room::{self, Beat, BeatReport, Reported, RoomState, RoomStep};
use interview_engine::{AnswerEnd, EngineError, FinishReason, InterviewConfig, InterviewPlan};
use interview_llm::{ChatModel, ChatRequest, ChatStream, LlmError, Pricing};
use interview_platform::http::{ErrorBody, Json, Path, serve_blob};
use interview_platform::tasks::{NewTask, Task, TaskQueue, TaskRegistry, TaskResult};
use interview_platform::usage::{self, LlmUsage};
use interview_platform::{ApiError, ApiResult};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgConnection;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::Deps;
use crate::access::RoomAccess;
use crate::events::{self, EventSource, NewEvent};
use crate::protocol::{AnswerSettings, NextStep, Progress, Utterance};
use crate::session::{consent_agreed, parse_config};

/// 候选人中途离开的面试由这个任务收掉。
pub const WATCHDOG_TASK: &str = "room.watchdog";

/// 页面报上来的时刻最多比服务端的时钟快这么多，再多就按服务端的算。
const CLOCK_SLACK_MS: i64 = 5_000;

pub(crate) fn router() -> OpenApiRouter<Deps> {
    OpenApiRouter::new()
        .routes(routes!(start_interview))
        .routes(routes!(advance_interview))
        .routes(routes!(room_utterance_audio))
        .routes(routes!(finish_interview))
}

pub(crate) fn register_tasks(registry: &mut TaskRegistry, deps: Deps) {
    registry.register_fn(WATCHDOG_TASK, move |task| {
        let deps = deps.clone();
        async move { watchdog_task(&deps, task).await }
    });
}

// ───────────────────────── 读写面试的状态 ─────────────────────────

#[derive(Debug, sqlx::FromRow)]
struct Row {
    status: InterviewStatus,
    plan: Option<Value>,
    config: Value,
    runtime_state: Option<Value>,
    started_at: Option<DateTime<Utc>>,
    end_reason: Option<String>,
}

impl Row {
    fn is_finished(&self) -> bool {
        matches!(
            self.status,
            InterviewStatus::Completed | InterviewStatus::Abandoned
        )
    }

    /// 结束了的面试，页面该看到的结束原因。
    fn finish_step(&self) -> NextStep {
        NextStep::Finish {
            reason: self
                .end_reason
                .clone()
                .unwrap_or_else(|| finish_reason(FinishReason::Completed).to_owned()),
        }
    }

    /// 面试已经进行了多少毫秒。
    fn elapsed_ms(&self, now: DateTime<Utc>) -> i64 {
        self.started_at
            .map(|started| (now - started).num_milliseconds().max(0))
            .unwrap_or(0)
    }
}

/// 读出一场面试。`lock` 为真时锁住这一行，到事务结束为止别的请求改不了它。
async fn load(conn: &mut PgConnection, interview_id: Uuid, lock: bool) -> Result<Row, sqlx::Error> {
    let sql = if lock {
        "SELECT status, plan, config, runtime_state, started_at, end_reason
           FROM interviews WHERE id = $1 FOR UPDATE"
    } else {
        "SELECT status, plan, config, runtime_state, started_at, end_reason
           FROM interviews WHERE id = $1"
    };
    sqlx::query_as(sql).bind(interview_id).fetch_one(conn).await
}

/// 这场面试的题目和配置。
fn script(interview_id: Uuid, row: &Row) -> ApiResult<(InterviewPlan, InterviewConfig)> {
    let Some(raw) = row.plan.clone().filter(|plan| !plan.is_null()) else {
        return Err(
            ApiError::conflict("面试还在准备中，请稍后再打开").with_code("interview_not_ready")
        );
    };
    let plan: InterviewPlan = serde_json::from_value(raw).map_err(|err| {
        ApiError::internal(anyhow::anyhow!("面试 {interview_id} 的题目读不出来：{err}"))
    })?;
    Ok((plan, parse_config(interview_id, row.config.clone())))
}

/// 存在库里的面试间状态；还没开始过是 `None`。
fn stored_state(interview_id: Uuid, row: &Row) -> ApiResult<Option<RoomState>> {
    let Some(raw) = row.runtime_state.clone().filter(|state| !state.is_null()) else {
        return Ok(None);
    };
    serde_json::from_value(raw).map(Some).map_err(|err| {
        ApiError::internal(anyhow::anyhow!("面试 {interview_id} 的进度读不出来：{err}"))
    })
}

async fn save_state(
    conn: &mut PgConnection,
    interview_id: Uuid,
    state: &RoomState,
) -> ApiResult<()> {
    let raw = serde_json::to_value(state).map_err(ApiError::internal)?;
    sqlx::query("UPDATE interviews SET runtime_state = $2 WHERE id = $1")
        .bind(interview_id)
        .bind(raw)
        .execute(conn)
        .await?;
    Ok(())
}

fn engine_error(err: EngineError) -> ApiError {
    match err {
        EngineError::InvalidState(message) => ApiError::conflict(message).with_code("seq_conflict"),
        EngineError::InvalidInput(message) => ApiError::invalid_field("end_reason", message),
        other => ApiError::internal(other),
    }
}

/// 结束原因在协议里的写法。
fn finish_reason(reason: FinishReason) -> &'static str {
    match reason {
        FinishReason::Completed => "completed",
        FinishReason::TimeUp => "time_up",
        FinishReason::PoorAnswers => "poor_answers",
        FinishReason::NoResponse => "no_response",
        FinishReason::CandidateRequest => "candidate_request",
    }
}

/// 结束面试：改状态、记原因，同一个事务里把「转写和出报告」交出去。
async fn close(
    conn: &mut PgConnection,
    interview_id: Uuid,
    status: InterviewStatus,
    reason: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE interviews SET status = $2, ended_at = now(), end_reason = $3
          WHERE id = $1 AND status = 'in_progress'",
    )
    .bind(interview_id)
    .bind(status)
    .bind(reason)
    .execute(&mut *conn)
    .await?;
    TaskQueue::enqueue_in(
        conn,
        NewTask::new(
            task_kind::INTERVIEW_FINALIZE,
            json!({ "interview_id": interview_id }),
        )
        .dedupe(format!("interview.finalize:{interview_id}")),
    )
    .await?;
    Ok(())
}

/// 候选人不在了（自己点了离开，或者断线没回来）时怎么收场。
///
/// 结束语已经发出去了的，算正常结束：题都问完了，只是没等到页面报告结束语播完。
async fn close_unattended(
    conn: &mut PgConnection,
    interview_id: Uuid,
    state: Option<&RoomState>,
    reason: &str,
) -> Result<(), sqlx::Error> {
    match state.and_then(|state| state.session.current.finished) {
        Some(natural) => {
            close(
                conn,
                interview_id,
                InterviewStatus::Completed,
                finish_reason(natural),
            )
            .await
        }
        None => close(conn, interview_id, InterviewStatus::Abandoned, reason).await,
    }
}

// ───────────────────────── 给页面的「下一步」 ─────────────────────────

/// 把引擎给的一拍变成协议里的「下一步」：补上语音地址和时长。没有合成出语音的句子地址是空的。
async fn decorate(deps: &Deps, interview_id: Uuid, step: &RoomStep) -> ApiResult<NextStep> {
    let beat = match step {
        RoomStep::Finish { reason } => {
            return Ok(NextStep::Finish {
                reason: finish_reason(*reason).to_owned(),
            });
        }
        RoomStep::Say(beat) => beat,
    };
    let mut keys = Vec::with_capacity(beat.preload_keys.len() + 1);
    keys.push(beat.utterance.key.clone());
    keys.extend(beat.preload_keys.iter().cloned());
    let voiced: Vec<(String, Option<i32>)> = sqlx::query_as(
        "SELECT key, audio_duration_ms FROM interview_utterances
          WHERE interview_id = $1 AND key = ANY($2)
            AND audio_blob_key IS NOT NULL AND synth_status = 'done'",
    )
    .bind(interview_id)
    .bind(&keys)
    .fetch_all(&deps.ctx.db)
    .await?;
    let duration_of = |key: &str| {
        voiced
            .iter()
            .find(|(voiced_key, _)| voiced_key == key)
            .map(|(_, duration)| duration.map(i64::from))
    };

    let own = duration_of(&beat.utterance.key);
    Ok(NextStep::Say {
        seq: beat.seq,
        utterance: Utterance {
            key: beat.utterance.key.clone(),
            kind: utterance_kind(beat.utterance.kind),
            text: beat.utterance.text.clone(),
            audio_url: own.map(|_| audio_url(&beat.utterance.key)),
            audio_duration_ms: own.flatten(),
        },
        expects_answer: beat.expects_answer,
        answer: beat.answer.map(|window| AnswerSettings {
            silence_sec: window.silence_sec,
            max_sec: window.max_sec,
            min_sec: window.min_sec,
        }),
        progress: Progress {
            index: beat.progress.index,
            total: beat.progress.total,
            elapsed_ms: clamp_i64(beat.progress.elapsed_ms),
            expected_total_ms: clamp_i64(beat.progress.expected_total_ms),
        },
        preload_audio_urls: beat
            .preload_keys
            .iter()
            .filter(|key| duration_of(key).is_some())
            .map(|key| audio_url(key))
            .collect(),
    })
}

/// 引擎和平台各有一份句子的类别，取值相同。
fn utterance_kind(kind: interview_engine::UtteranceKind) -> UtteranceKind {
    use interview_engine::UtteranceKind as Engine;
    match kind {
        Engine::Opening => UtteranceKind::Opening,
        Engine::Question => UtteranceKind::Question,
        Engine::FollowUp => UtteranceKind::FollowUp,
        Engine::Transition => UtteranceKind::Transition,
        Engine::Closing => UtteranceKind::Closing,
        Engine::Other => UtteranceKind::Other,
    }
}

fn audio_url(key: &str) -> String {
    format!("/api/v1/room/utterances/{key}/audio")
}

fn clamp_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// 续面时该播的那一句：面试进行中才有。
pub(crate) async fn resume_step(deps: &Deps, access: &RoomAccess) -> ApiResult<Option<NextStep>> {
    if access.status != InterviewStatus::InProgress {
        return Ok(None);
    }
    let mut conn = deps.ctx.db.acquire().await?;
    let row = load(&mut conn, access.interview_id, false).await?;
    drop(conn);
    match stored_state(access.interview_id, &row)? {
        Some(state) => Ok(Some(
            decorate(deps, access.interview_id, &state.last).await?,
        )),
        None => Ok(None),
    }
}

// ───────────────────────── 开始 ─────────────────────────

/// 开始面试
///
/// 返回第一句话。面试已经开始时（刷新页面、换了设备）返回当前该播的那一句，从这里接着面；
/// 已经结束时返回 `finish`。还没同意必须同意的事项返回 409（`consent_required`）。
#[utoipa::path(
    post,
    path = "/api/v1/room/start",
    tag = "room",
    security(("invite_token" = [])),
    responses(
        (status = 200, description = "该播的那一句", body = NextStep),
        (status = 409, description = "还没同意，或面试还在准备", body = ErrorBody),
    )
)]
async fn start_interview(
    State(deps): State<Deps>,
    access: RoomAccess,
) -> ApiResult<Json<NextStep>> {
    let interview_id = access.interview_id;
    if access.status == InterviewStatus::Invited && !consent_agreed(&deps, &access).await? {
        return Err(ApiError::conflict("请先确认同意事项").with_code("consent_required"));
    }

    let mut tx = deps.ctx.db.begin().await?;
    let row = load(&mut tx, interview_id, true).await?;
    if row.is_finished() {
        return Ok(Json(row.finish_step()));
    }
    let step = match (row.status, stored_state(interview_id, &row)?) {
        (InterviewStatus::InProgress, Some(state)) => state.last,
        (InterviewStatus::Invited | InterviewStatus::InProgress, _) => {
            let (plan, config) = script(interview_id, &row)?;
            let (state, step) = room::open(&plan, &config).map_err(engine_error)?;
            save_state(&mut tx, interview_id, &state).await?;
            sqlx::query(
                "UPDATE interviews
                    SET status = 'in_progress', started_at = COALESCE(started_at, now())
                  WHERE id = $1",
            )
            .bind(interview_id)
            .execute(&mut *tx)
            .await?;
            let first_check = Utc::now()
                + chrono::Duration::minutes(i64::from(deps.config.abandon_after_minutes));
            TaskQueue::enqueue_in(
                &mut tx,
                NewTask::new(WATCHDOG_TASK, json!({ "interview_id": interview_id }))
                    .dedupe(format!("{WATCHDOG_TASK}:{interview_id}"))
                    .run_at(first_check),
            )
            .await?;
            tracing::info!(%interview_id, "面试开始");
            step
        }
        _ => {
            return Err(
                ApiError::conflict("面试还在准备中，请稍后再打开").with_code("interview_not_ready")
            );
        }
    };
    tx.commit().await?;
    Ok(Json(decorate(&deps, interview_id, &step).await?))
}

// ───────────────────────── 推进 ─────────────────────────

/// 候选人这一段回答是怎么结束的。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnswerEndReason {
    /// 开口之后静音够久了
    Silence,
    /// 点了「答完了」
    Button,
    /// 到了最长作答时间
    Timeout,
    /// 一直没开口
    NoSpeech,
}

impl AnswerEndReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Silence => "silence",
            Self::Button => "button",
            Self::Timeout => "timeout",
            Self::NoSpeech => "no_speech",
        }
    }

    fn to_engine(self) -> AnswerEnd {
        match self {
            Self::Silence => AnswerEnd::Silence,
            Self::Button => AnswerEnd::Manual,
            Self::Timeout => AnswerEnd::MaxDuration,
            Self::NoSpeech => AnswerEnd::NoSpeech,
        }
    }
}

/// 页面上报：一句话播完了；要听回答的那一句，连同回答一起报。
///
/// 时刻都是面试时间轴上的毫秒数（从面试开始算起）。
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct AdvanceRequest {
    /// 上报的是哪一句（「下一步」里的 `seq`）
    pub seq: u32,
    /// 这一句的 key，用来核对页面和服务端说的是同一句
    pub utterance_key: String,
    pub playback_start_ms: Option<i64>,
    pub playback_end_ms: Option<i64>,
    /// 候选人开口的时刻；不需要回答或者没开口是 null
    pub answer_start_ms: Option<i64>,
    pub answer_end_ms: Option<i64>,
    /// 不需要回答的句子是 null
    pub end_reason: Option<AnswerEndReason>,
    /// 候选人重听了几次
    #[serde(default)]
    pub replays: u32,
}

/// 一次推进的结果。
enum Advanced {
    /// 把这一句发给页面。
    Step(RoomStep),
    /// 面试已经结束。
    Finished(NextStep),
    /// 候选人答完了一段，要判断。`ack` 有值就先把这句过渡语发给页面。
    Answered { ack: Option<RoomStep> },
    /// 过渡语播完了，回答还没判断完。
    Waiting,
}

/// 推进面试
///
/// 页面每播完一句调一次；要听回答的那一句，等候选人答完再调，带上回答的起止时刻和结束原因。
/// 返回下一步。同一个 `seq` 重复提交返回同样的结果（断网重试用）；`seq` 既不是当前这一句
/// 也不是上一句时返回 409（`seq_conflict`）。面试已经结束时返回 `finish`。
#[utoipa::path(
    post,
    path = "/api/v1/room/advance",
    tag = "room",
    security(("invite_token" = [])),
    request_body = AdvanceRequest,
    responses(
        (status = 200, description = "下一步", body = NextStep),
        (status = 409, description = "面试还没开始，或 seq 对不上", body = ErrorBody),
        (status = 422, description = "要听回答的那一句没带结束原因", body = ErrorBody),
        (status = 503, description = "上一段回答还在处理，稍后重试", body = ErrorBody),
    )
)]
async fn advance_interview(
    State(deps): State<Deps>,
    access: RoomAccess,
    Json(request): Json<AdvanceRequest>,
) -> ApiResult<Json<NextStep>> {
    let interview_id = access.interview_id;
    // 最多走三遍：判断完一段回答之后要重新看一次状态，才知道该给页面什么
    for _ in 0..3 {
        match advance_once(&deps, interview_id, &request).await? {
            Advanced::Finished(step) => return Ok(Json(step)),
            Advanced::Step(step) => return Ok(Json(decorate(&deps, interview_id, &step).await?)),
            Advanced::Answered { ack: Some(ack) } => {
                // 过渡语先播着，回答放到后台判断。页面报告过渡语播完时判断多半已经好了
                let background = deps.clone();
                tokio::spawn(async move {
                    if let Err(err) = settle_pending(&background, interview_id).await {
                        tracing::warn!(%interview_id, reason = %err, "后台判断回答没成功，等页面下次上报时再试");
                    }
                });
                return Ok(Json(decorate(&deps, interview_id, &ack).await?));
            }
            Advanced::Answered { ack: None } => {
                if let Some(step) = settle_pending(&deps, interview_id).await? {
                    return Ok(Json(decorate(&deps, interview_id, &step).await?));
                }
            }
            Advanced::Waiting => {
                settle_pending(&deps, interview_id).await?;
            }
        }
    }
    Err(ApiError::unavailable("上一段回答还在处理，请稍后再试").with_code("answer_pending"))
}

async fn advance_once(
    deps: &Deps,
    interview_id: Uuid,
    request: &AdvanceRequest,
) -> ApiResult<Advanced> {
    let mut tx = deps.ctx.db.begin().await?;
    let row = load(&mut tx, interview_id, true).await?;
    if row.is_finished() {
        return Ok(Advanced::Finished(row.finish_step()));
    }
    let state = match (row.status, stored_state(interview_id, &row)?) {
        (InterviewStatus::InProgress, Some(state)) => state,
        _ => return Err(ApiError::conflict("面试还没有开始")),
    };
    let (plan, _config) = script(interview_id, &row)?;

    let limit = row.elapsed_ms(Utc::now());
    let times = BeatTimes::from_request(request, limit);
    let is_current = request.seq == state.seq;
    if is_current
        && let RoomStep::Say(beat) = &state.last
        && beat.utterance.key != request.utterance_key
    {
        return Err(
            ApiError::conflict("上报的这一句和服务端发出的对不上，请刷新页面")
                .with_code("seq_conflict"),
        );
    }

    let report = BeatReport {
        seq: request.seq,
        elapsed_ms: u64::try_from(times.end_ms()).unwrap_or(0),
        end_reason: request.end_reason.map(AnswerEndReason::to_engine),
        answer_ms: u64::try_from(times.answer_ms()).unwrap_or(0),
    };
    let reported = room::report(&plan, &state, &report).map_err(engine_error)?;

    if is_current && let RoomStep::Say(beat) = &state.last {
        record_turns(&mut tx, interview_id, &state, beat, request, &times).await?;
    }
    let outcome = match reported {
        Reported::Next { state: next, step } => {
            if next != state {
                save_state(&mut tx, interview_id, &next).await?;
            }
            if let RoomStep::Finish { reason } = &step {
                close(
                    &mut tx,
                    interview_id,
                    InterviewStatus::Completed,
                    finish_reason(*reason),
                )
                .await?;
                tracing::info!(%interview_id, reason = finish_reason(*reason), "面试结束");
            }
            Advanced::Step(step)
        }
        Reported::Answered {
            state: next, ack, ..
        } => {
            if next != state {
                save_state(&mut tx, interview_id, &next).await?;
            }
            Advanced::Answered { ack }
        }
        Reported::Waiting { .. } => Advanced::Waiting,
    };
    tx.commit().await?;
    Ok(outcome)
}

/// 页面报上来的几个时刻，已经收进合理的范围：不早于 0，不晚于服务端认为的现在。
struct BeatTimes {
    /// 服务端认为面试现在进行到了多少毫秒
    now_ms: i64,
    playback_start_ms: Option<i64>,
    playback_end_ms: Option<i64>,
    answer_start_ms: Option<i64>,
    answer_end_ms: Option<i64>,
}

impl BeatTimes {
    fn from_request(request: &AdvanceRequest, now_ms: i64) -> Self {
        let clamp = |value: Option<i64>| value.map(|ms| ms.clamp(0, now_ms + CLOCK_SLACK_MS));
        Self {
            now_ms,
            playback_start_ms: clamp(request.playback_start_ms),
            playback_end_ms: clamp(request.playback_end_ms),
            answer_start_ms: clamp(request.answer_start_ms),
            answer_end_ms: clamp(request.answer_end_ms),
        }
    }

    /// 这一句（连同回答）结束的时刻。
    fn end_ms(&self) -> i64 {
        self.answer_end_ms
            .or(self.playback_end_ms)
            .unwrap_or(self.now_ms)
    }

    fn answer_ms(&self) -> i64 {
        match (self.answer_start_ms, self.answer_end_ms) {
            (Some(start), Some(end)) => (end - start).max(0),
            _ => 0,
        }
    }
}

/// 面试官这一句在对话记录里的序号。每一拍占两个位置：面试官的话在前，候选人的回答在后。
fn interviewer_turn_seq(beat_seq: u32) -> i32 {
    i32::try_from(beat_seq.saturating_mul(2).saturating_sub(1)).unwrap_or(i32::MAX)
}

fn candidate_turn_seq(beat_seq: u32) -> i32 {
    i32::try_from(beat_seq.saturating_mul(2)).unwrap_or(i32::MAX)
}

/// 把刚播完的这一句和候选人的回答记进对话。重复上报不会记两遍。
///
/// 候选人说了什么这时还不知道，文字先空着：要追问时由 [`settle_pending`] 填，
/// 面试结束后整段重新转写时再补全。
async fn record_turns(
    conn: &mut PgConnection,
    interview_id: Uuid,
    state: &RoomState,
    beat: &Beat,
    request: &AdvanceRequest,
    times: &BeatTimes,
) -> Result<(), sqlx::Error> {
    let said_at = times
        .playback_start_ms
        .or(times.playback_end_ms)
        .unwrap_or(times.now_ms);
    sqlx::query(
        "INSERT INTO interview_turns
                (id, interview_id, seq, speaker, utterance_key, text, start_ms, end_ms)
         VALUES ($1, $2, $3, 'interviewer', $4, $5, $6, $7)
         ON CONFLICT (interview_id, seq) DO NOTHING",
    )
    .bind(interview_core::new_id())
    .bind(interview_id)
    .bind(interviewer_turn_seq(beat.seq))
    .bind(&beat.utterance.key)
    .bind(&beat.utterance.text)
    .bind(said_at)
    .bind(times.playback_end_ms)
    .execute(&mut *conn)
    .await?;

    let Some(end_reason) = request.end_reason else {
        return Ok(());
    };
    if !beat.expects_answer || end_reason == AnswerEndReason::NoSpeech {
        return Ok(());
    }
    // 回答算在哪句话下面由引擎说了算（重问一遍时，回答的还是原来那道题）
    let answered_key = state
        .session
        .current
        .listen
        .as_ref()
        .map(|listen| listen.utterance_key.as_str())
        .unwrap_or(beat.utterance.key.as_str());
    let answered_at = times
        .answer_start_ms
        .or(times.playback_end_ms)
        .unwrap_or(times.now_ms);
    sqlx::query(
        "INSERT INTO interview_turns
                (id, interview_id, seq, speaker, utterance_key, start_ms, end_ms, end_reason)
         VALUES ($1, $2, $3, 'candidate', $4, $5, $6, $7)
         ON CONFLICT (interview_id, seq) DO NOTHING",
    )
    .bind(interview_core::new_id())
    .bind(interview_id)
    .bind(candidate_turn_seq(beat.seq))
    .bind(answered_key)
    .bind(answered_at)
    .bind(times.answer_end_ms)
    .bind(end_reason.as_str())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

// ───────────────────────── 判断回答 ─────────────────────────

/// 没配大模型时的占位。不带文字去判断时引擎不会调模型，所以它不会真的被用到。
struct NoModel;

#[async_trait]
impl ChatModel for NoModel {
    async fn chat_stream(&self, _request: ChatRequest) -> Result<ChatStream, LlmError> {
        Err(LlmError::Config("没有配置大模型".to_owned()))
    }
}

/// 判断等着的那段回答，把结果存下来。
///
/// 返回要马上发给页面的下一句。过渡语还在播、没有回答在等、或者别的请求已经判断完了，
/// 都返回 `None`，调用方重新看一次状态就知道该怎么办。重复调用是安全的。
async fn settle_pending(deps: &Deps, interview_id: Uuid) -> ApiResult<Option<RoomStep>> {
    let _one_at_a_time = deps.settle_lock(interview_id).lock().await;

    let mut conn = deps.ctx.db.acquire().await?;
    let row = load(&mut conn, interview_id, false).await?;
    drop(conn);
    if row.status != InterviewStatus::InProgress {
        return Ok(None);
    }
    let Some(state) = stored_state(interview_id, &row)? else {
        return Ok(None);
    };
    let Some(pending) = state.pending() else {
        return Ok(None);
    };
    let (plan, config) = script(interview_id, &row)?;

    // 识别加判断一共最多等这么久
    let started = Instant::now();
    let wait = Duration::from_secs(u64::from(deps.config.settle_timeout_sec));
    let deadline = tokio::time::Instant::now() + wait;

    let answer = match (&deps.llm, pending.wants_transcript) {
        (Some(_), true) => transcribe_answer(deps, interview_id, deadline).await?,
        _ => None,
    };
    let transcript = answer.as_ref().map(|answer| answer.text.clone());
    let model: Arc<dyn ChatModel> = deps.llm.clone().unwrap_or_else(|| Arc::new(NoModel));

    let judged = tokio::time::timeout_at(
        deadline,
        room::settle(model.as_ref(), &plan, &config, &state, transcript),
    )
    .await;
    let settled = match judged {
        Ok(Ok(settled)) => settled,
        other => {
            // 模型出错或者太慢：不让候选人干等，这一段不判断，按顺序问下去
            let reason = match other {
                Ok(Err(err)) => err.to_string(),
                _ => format!("等了 {} 秒模型还没回来", wait.as_secs()),
            };
            tracing::warn!(%interview_id, %reason, "这段回答没判断成，按顺序问下一题");
            room::settle(&NoModel, &plan, &config, &state, None)
                .await
                .map_err(engine_error)?
        }
    };
    let latency = started.elapsed();

    // 判断的这几秒里没拿着锁。写之前确认状态还是判断前的样子，不然就是别的进程先写了
    let mut tx = deps.ctx.db.begin().await?;
    let row = load(&mut tx, interview_id, true).await?;
    if row.status != InterviewStatus::InProgress
        || stored_state(interview_id, &row)?.as_ref() != Some(&state)
    {
        return Ok(None);
    }
    save_state(&mut tx, interview_id, &settled.state).await?;
    if let Some(answer) = &answer {
        sqlx::query(
            "UPDATE interview_turns SET text = $3, transcript_source = 'live'
              WHERE interview_id = $1 AND seq = $2 AND speaker = 'candidate'",
        )
        .bind(interview_id)
        .bind(answer.turn_seq)
        .bind(&answer.text)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    if settled.usage.total_tokens() > 0 {
        let thinking = serde_json::to_value(config.live_thinking)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned));
        let model_name = if plan.generation.model.is_empty() {
            interview_llm::DEFAULT_MODEL
        } else {
            plan.generation.model.as_str()
        };
        usage::record(
            &deps.ctx.db,
            LlmUsage {
                purpose: "interview.turn",
                model: model_name,
                thinking: thinking.as_deref(),
                prompt_tokens: settled.usage.prompt_tokens,
                completion_tokens: settled.usage.completion_tokens,
                reasoning_tokens: settled.usage.reasoning_tokens,
                cached_tokens: settled.usage.cache_hit_tokens,
                cost_cny: Some(Pricing::FLASH_PEAK.cost_cny(&settled.usage)),
                latency_ms: Some(u64::try_from(latency.as_millis()).unwrap_or(u64::MAX)),
                interview_id: Some(interview_id),
                task_id: None,
            },
        )
        .await;
    }
    Ok(settled.step)
}

/// 候选人刚答完的那一段的文字，以及它在对话记录里的序号。
struct AnswerText {
    turn_seq: i32,
    text: String,
}

/// 把候选人最近的一段回答转成文字。没接语音识别、这一段没有起止时刻、识别不出来，都是 `None`。
async fn transcribe_answer(
    deps: &Deps,
    interview_id: Uuid,
    deadline: tokio::time::Instant,
) -> ApiResult<Option<AnswerText>> {
    let Some(transcriber) = &deps.transcriber else {
        return Ok(None);
    };
    let latest: Option<(i32, i64, Option<i64>)> = sqlx::query_as(
        "SELECT seq, start_ms, end_ms FROM interview_turns
          WHERE interview_id = $1 AND speaker = 'candidate'
          ORDER BY seq DESC LIMIT 1",
    )
    .bind(interview_id)
    .fetch_optional(&deps.ctx.db)
    .await?;
    let Some((turn_seq, start_ms, Some(end_ms))) = latest else {
        return Ok(None);
    };
    let text = tokio::time::timeout_at(
        deadline,
        transcriber.answer_text(interview_id, start_ms, end_ms),
    )
    .await
    .ok()
    .flatten()
    .map(|text| text.trim().to_owned())
    .filter(|text| !text.is_empty());
    Ok(text.map(|text| AnswerText { turn_seq, text }))
}

// ───────────────────────── 面试官语音 ─────────────────────────

/// 面试官语音
///
/// 「下一步」里 `audio_url` 和 `preload_audio_urls` 指向这里。只能取当前这一句、服务端让提前取的
/// 那几句和已经播过的句子，别的返回 404：还没问到的题不能提前听。支持 `Range`。
#[utoipa::path(
    get,
    path = "/api/v1/room/utterances/{key}/audio",
    tag = "room",
    security(("invite_token" = [])),
    params(("key" = String, Path, description = "这句话的 key")),
    responses(
        (status = 200, description = "音频", content_type = "audio/wav"),
        (status = 404, description = "没有这句话的语音，或者现在还不能取", body = ErrorBody),
        (status = 409, description = "面试不在进行中", body = ErrorBody),
    )
)]
async fn room_utterance_audio(
    State(deps): State<Deps>,
    access: RoomAccess,
    Path(key): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    access.require_in_progress()?;
    let interview_id = access.interview_id;
    let mut conn = deps.ctx.db.acquire().await?;
    let row = load(&mut conn, interview_id, false).await?;
    drop(conn);
    let offered = match stored_state(interview_id, &row)?.map(|state| state.last) {
        Some(RoomStep::Say(beat)) => beat.utterance.key == key || beat.preload_keys.contains(&key),
        _ => false,
    };
    let (already_said,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM interview_turns
                         WHERE interview_id = $1 AND speaker = 'interviewer' AND utterance_key = $2)",
    )
    .bind(interview_id)
    .bind(&key)
    .fetch_one(&deps.ctx.db)
    .await?;
    if !offered && !already_said {
        return Err(ApiError::not_found("语音"));
    }

    let audio: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT audio_blob_key, audio_content_type FROM interview_utterances
          WHERE interview_id = $1 AND key = $2
            AND audio_blob_key IS NOT NULL AND synth_status = 'done'",
    )
    .bind(interview_id)
    .bind(&key)
    .fetch_optional(&deps.ctx.db)
    .await?;
    let Some((blob_key, content_type)) = audio else {
        return Err(ApiError::not_found("语音"));
    };
    serve_blob(
        deps.ctx.blobs.as_ref(),
        &blob_key,
        content_type.as_deref().unwrap_or("audio/wav"),
        &headers,
    )
    .await
}

// ───────────────────────── 结束 ─────────────────────────

/// 页面为什么说面试结束了。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FinishKind {
    /// 面试官说了结束（页面已经拿到 `finish`）
    Completed,
    /// 候选人自己点了「结束面试」
    CandidateLeft,
}

/// 结束面试的请求。
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct FinishRequest {
    pub reason: FinishKind,
}

/// 结束面试
///
/// 候选人中途点了「结束面试」时调，面试记成「中途离开」，已经录下的部分照样转写和出报告。
/// 面试正常问完时服务端自己会结束它，页面不用调；调了也没有副作用。重复调用没有副作用。
#[utoipa::path(
    post,
    path = "/api/v1/room/finish",
    tag = "room",
    security(("invite_token" = [])),
    request_body = FinishRequest,
    responses(
        (status = 204, description = "已结束"),
        (status = 409, description = "面试还没开始，或者还没问完却说正常结束", body = ErrorBody),
    )
)]
async fn finish_interview(
    State(deps): State<Deps>,
    access: RoomAccess,
    Json(request): Json<FinishRequest>,
) -> ApiResult<StatusCode> {
    let interview_id = access.interview_id;
    let mut tx = deps.ctx.db.begin().await?;
    let row = load(&mut tx, interview_id, true).await?;
    if row.is_finished() {
        return Ok(StatusCode::NO_CONTENT);
    }
    if row.status != InterviewStatus::InProgress {
        return Err(ApiError::conflict("面试还没有开始"));
    }
    match request.reason {
        FinishKind::Completed => {
            return Err(ApiError::conflict("面试还没有问完").with_code("interview_not_finished"));
        }
        FinishKind::CandidateLeft => {
            let state = stored_state(interview_id, &row)?;
            close_unattended(&mut tx, interview_id, state.as_ref(), "candidate_left").await?;
        }
    }
    tx.commit().await?;
    tracing::info!(%interview_id, "候选人结束了面试");
    Ok(StatusCode::NO_CONTENT)
}

// ───────────────────────── 候选人中途离开 ─────────────────────────

#[derive(Debug, Deserialize)]
struct WatchdogPayload {
    interview_id: Uuid,
}

/// 看一场进行中的面试还有没有动静。
///
/// 页面在的时候每隔几秒就传一片录像，所以「最近一次动静」取状态变化、录像分片、过程事件里最晚的。
/// 超过设定的时间没有动静，就当候选人不会回来了，结束这场面试；不然到时候再看一次。
async fn watchdog_task(deps: &Deps, task: Task) -> TaskResult {
    let payload: WatchdogPayload = task.payload_as()?;
    let interview_id = payload.interview_id;

    let mut tx = deps.ctx.db.begin().await?;
    let found: Option<(InterviewStatus,)> =
        sqlx::query_as("SELECT status FROM interviews WHERE id = $1 FOR UPDATE")
            .bind(interview_id)
            .fetch_optional(&mut *tx)
            .await?;
    if !matches!(found, Some((InterviewStatus::InProgress,))) {
        return Ok(Some(json!({ "outcome": "not_in_progress" })));
    }
    let (last_activity,): (DateTime<Utc>,) = sqlx::query_as(
        "SELECT GREATEST(
                    i.updated_at,
                    (SELECT max(c.received_at)
                       FROM interview_recording_chunks c
                       JOIN interview_recordings r ON r.id = c.recording_id
                      WHERE r.interview_id = i.id),
                    (SELECT max(e.created_at) FROM interview_events e WHERE e.interview_id = i.id))
           FROM interviews i WHERE i.id = $1",
    )
    .bind(interview_id)
    .fetch_one(&mut *tx)
    .await?;

    let now = Utc::now();
    let deadline =
        last_activity + chrono::Duration::minutes(i64::from(deps.config.abandon_after_minutes));
    if deadline > now {
        // 这个任务自己还占着原来的去重键，下一次检查用带时间的键
        TaskQueue::enqueue_in(
            &mut tx,
            NewTask::new(WATCHDOG_TASK, json!({ "interview_id": interview_id }))
                .dedupe(format!(
                    "{WATCHDOG_TASK}:{interview_id}:{}",
                    deadline.timestamp()
                ))
                .run_at(deadline + chrono::Duration::seconds(5)),
        )
        .await?;
        tx.commit().await?;
        return Ok(Some(json!({ "outcome": "still_active" })));
    }

    let row = load(&mut tx, interview_id, false).await?;
    let state = stored_state(interview_id, &row).ok().flatten();
    close_unattended(&mut tx, interview_id, state.as_ref(), "connection_lost").await?;
    events::insert(
        &mut *tx,
        interview_id,
        &[NewEvent {
            kind: "interview_abandoned".to_owned(),
            at_ms: Some(row.elapsed_ms(last_activity)),
            occurred_at: now,
            source: EventSource::Server,
            payload: json!({ "idle_minutes": deps.config.abandon_after_minutes }),
        }],
    )
    .await?;
    tx.commit().await?;
    tracing::info!(%interview_id, "候选人离开后没有回来，面试结束");
    Ok(Some(json!({ "outcome": "closed" })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(end: Option<i64>) -> AdvanceRequest {
        AdvanceRequest {
            seq: 1,
            utterance_key: "q1".to_owned(),
            playback_start_ms: Some(1_000),
            playback_end_ms: Some(4_000),
            answer_start_ms: Some(5_000),
            answer_end_ms: end,
            end_reason: Some(AnswerEndReason::Silence),
            replays: 0,
        }
    }

    #[test]
    fn times_from_the_page_are_kept_within_the_server_clock() {
        // 页面说回答在第 10 分钟结束，服务端看面试才开始 20 秒
        let times = BeatTimes::from_request(&request(Some(600_000)), 20_000);
        assert_eq!(times.end_ms(), 20_000 + CLOCK_SLACK_MS);
        assert_eq!(times.answer_ms(), 20_000);

        let negative = AdvanceRequest {
            playback_start_ms: Some(-50),
            ..request(Some(9_000))
        };
        let times = BeatTimes::from_request(&negative, 20_000);
        assert_eq!(times.playback_start_ms, Some(0));
        assert_eq!(times.end_ms(), 9_000);
        assert_eq!(times.answer_ms(), 4_000);
    }

    #[test]
    fn a_beat_without_an_answer_ends_when_playback_ends() {
        let silent = AdvanceRequest {
            answer_start_ms: None,
            answer_end_ms: None,
            end_reason: None,
            ..request(None)
        };
        let times = BeatTimes::from_request(&silent, 20_000);
        assert_eq!(times.end_ms(), 4_000);
        assert_eq!(times.answer_ms(), 0);
    }

    #[test]
    fn each_beat_takes_two_places_in_the_conversation() {
        assert_eq!(interviewer_turn_seq(1), 1);
        assert_eq!(candidate_turn_seq(1), 2);
        assert_eq!(interviewer_turn_seq(7), 13);
        assert_eq!(candidate_turn_seq(7), 14);
    }
}
