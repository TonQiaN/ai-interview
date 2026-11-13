//! 实时对话：候选人说完一段，面试官现场接话（追问、回应，或者进入下一题）。
//!
//! 怎么接话由 `interview_engine::live` 决定。页面和服务端之间还是逐题模式的那套办法：服务端一次
//! 给一句，页面播完（要听回答的那一句等候选人答完）上报一次，拿下一句。所以录制、断网重试、
//! 刷新续面都不用另做一套。和逐题模式不一样的地方在服务端：
//!
//! 1. 候选人答完一段，服务端把这段话转成文字（用的是上传上来的录像分片），交给引擎。
//! 2. 引擎边想边出结果。提前合成好的句子（过渡语、下一题）直接排进队列；现场生成的追问和回应
//!    按句子送去合成，合成好一句排一句。这样很短的接话（「明白。」）可以先播，后面的问句在它
//!    播的时候合成。
//! 3. 页面的上报在队列里有东西时马上返回，没有就等，等到有为止。
//!
//! 状态存在 `interviews.runtime_state` 里（[`RtState`]），每排进一句就写一次库：进程重启后，
//! 已经排好的句子还在，没想完的那一轮重新想一遍。谁在等、哪场面试正在想，记在进程内存里
//! （[`Hub`]），所以和现场转写一样，一场面试要由同一个进程服务到底。
//!
//! 候选人说的话只进数据库，不进日志。

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use chrono::Utc;
use interview_core::blob_key;
use interview_core::status::InterviewStatus;
use interview_engine::live::{self, CandidateTurn, LiveAction, LiveEvent, LiveOutcome, LiveState};
use interview_engine::room::AnswerWindow;
use interview_engine::session::{Listen, Progress as EngineProgress};
use interview_engine::{
    EngineError, FinishReason, InterviewConfig, InterviewMode, InterviewPlan, Language,
    Utterance as EngineUtterance, UtteranceKind as EngineKind,
};
use interview_llm::ChatModel;
use interview_platform::{ApiError, ApiResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgConnection;
use tokio::sync::{mpsc, watch};
use uuid::Uuid;

use crate::Deps;
use crate::conduct::{
    self, AdvanceRequest, AnswerEndReason, BeatTimes, NoModel, Row, at_elapsed, audio_url,
    clamp_i64, engine_error, finish_reason, utterance_kind,
};
use crate::protocol::{AnswerSettings, NextStep, Progress, Utterance};

/// 状态结构的版本。
pub const RT_SCHEMA_VERSION: u32 = 1;

/// 页面的一次上报最多等这么久。到时候还没有下一句就返回 503，页面会重试，后台接着想。
const ADVANCE_WAIT: Duration = Duration::from_secs(30);
/// 等下一句的时候，每隔这么久自己看一次库。平时靠通知，这是通知丢了时的保底。
const POLL_EVERY: Duration = Duration::from_secs(1);
/// 页面的一次上报里，一轮没想成最多重来几回。
const TURN_ATTEMPTS: u32 = 3;
/// 现场合成一句话最多等这么久，超过了这一句只显示文字。
const SYNTH_TIMEOUT: Duration = Duration::from_secs(20);
/// 实时对话里，至少听这么多秒才允许因为静音结束。候选人的话可以很短（「没有了」）。
const MIN_ANSWER_SEC: u32 = 1;
/// 候选人说了话但没转写出来时，交给引擎的文字。引擎会请候选人再说一遍。
const UNHEARD: &str = "（这一段没有听清）";
/// 现场生成的句子在 `interview_utterances.seq` 里从这里排起，排在提前准备的句子后面。
const LIVE_UTTERANCE_SEQ_BASE: i64 = 100_000;

// ───────────────────────── 状态 ─────────────────────────

/// 现场合成出来的语音存在哪。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LiveAudio {
    blob_key: String,
    duration_ms: i64,
}

/// 一拍：面试官说一句话。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RtBeat {
    /// 第几拍。排在队列里还没发出去的是 0，发出去时才定
    seq: u32,
    /// 这一拍说的话。现场生成的一段话按句子拆成几拍，几拍的 key 相同，文字是各自那一句
    utterance: EngineUtterance,
    /// 现场生成的话是其中的第几句（从 1 开始）；提前合成的句子是 `None`
    #[serde(default)]
    part: Option<u32>,
    /// 现场合成的语音；没合成出来，或者是提前合成的句子，是 `None`
    #[serde(default)]
    audio: Option<LiveAudio>,
    /// 有值表示播完之后要听候选人回答
    #[serde(default)]
    answer: Option<AnswerWindow>,
    progress: EngineProgress,
}

/// 最近发给页面的一步。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum RtStep {
    Say(RtBeat),
    Finish { reason: FinishReason },
}

/// 候选人答完的一段，面试官还没想好怎么接。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PendingTurn {
    /// 答的是第几拍
    beat_seq: u32,
    /// 这段话结束时面试进行了多久
    elapsed_ms: u64,
    answer_ms: u64,
    /// 候选人有没有开口
    spoke: bool,
}

/// 实时对话的状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RtState {
    schema_version: u32,
    /// 引擎的状态。一轮想完了才更新
    live: LiveState,
    /// 最近发出的是第几拍
    seq: u32,
    /// 最近发出的一步。重试和断点续面时原样返回
    last: RtStep,
    /// 已经准备好、还没发给页面的句子
    queue: Vec<RtBeat>,
    /// 有值表示正在想怎么接这段话，队列里还会有新的句子进来
    thinking: Option<PendingTurn>,
    /// 有值表示队列发完之后面试结束
    ending: Option<FinishReason>,
}

/// 存着的进度是不是实时对话的。逐题模式的状态里没有 `live` 这一项。
pub(crate) fn is_state(raw: &Value) -> bool {
    raw.get("live").is_some()
}

/// 读出存着的状态。
pub(crate) fn parse(interview_id: Uuid, row: &Row) -> ApiResult<Option<RtState>> {
    let Some(raw) = row.runtime_state.clone().filter(|state| !state.is_null()) else {
        return Ok(None);
    };
    serde_json::from_value(raw).map(Some).map_err(|err| {
        ApiError::internal(anyhow::anyhow!("面试 {interview_id} 的进度读不出来：{err}"))
    })
}

pub(crate) async fn save(
    conn: &mut PgConnection,
    interview_id: Uuid,
    state: &RtState,
) -> ApiResult<()> {
    let raw = serde_json::to_value(state).map_err(ApiError::internal)?;
    sqlx::query("UPDATE interviews SET runtime_state = $2 WHERE id = $1")
        .bind(interview_id)
        .bind(raw)
        .execute(conn)
        .await?;
    Ok(())
}

impl RtState {
    /// 结束语已经发出去了的，候选人不在了也算正常结束。
    pub(crate) fn natural_finish(&self) -> Option<FinishReason> {
        match &self.last {
            RtStep::Finish { reason } => Some(*reason),
            RtStep::Say(_) => self
                .ending
                .filter(|_| self.queue.is_empty() && self.thinking.is_none()),
        }
    }

    /// 页面现在能不能取这句话的语音：正在播的这一句，和已经排好的那几句。
    /// 现场合成的那几句连语音存在哪一起返回。
    pub(crate) fn offered(&self, key: &str, part: Option<u32>) -> Option<Option<&str>> {
        let current = match &self.last {
            RtStep::Say(beat) => Some(beat),
            RtStep::Finish { .. } => None,
        };
        current
            .into_iter()
            .chain(&self.queue)
            .find(|beat| beat.utterance.key == key && beat.part == part)
            .map(|beat| beat.audio.as_ref().map(|audio| audio.blob_key.as_str()))
    }
}

/// 这场面试能不能按实时对话进行：配置选了实时对话，而且大模型、语音识别、面试官的声音都在。
/// 少一样就按逐题进行，题目和语音是同一份。
pub(crate) async fn enabled(deps: &Deps, config: &InterviewConfig) -> bool {
    if config.mode != InterviewMode::Conversational {
        return false;
    }
    let (Some(_), Some(_), Some(transcriber)) = (&deps.llm, &deps.voice, &deps.transcriber) else {
        return false;
    };
    transcriber.available().await
}

/// 实时对话里怎么判断候选人说完了一段。
pub(crate) fn answer_window(
    deps: &Deps,
    silence_sec: u32,
    max_sec: u32,
    min_sec: u32,
) -> AnswerWindow {
    AnswerWindow {
        silence_sec: silence_sec.min(deps.config.realtime_silence_sec).max(1),
        max_sec,
        min_sec: min_sec.min(MIN_ANSWER_SEC),
    }
}

fn window(deps: &Deps, listen: &Listen) -> AnswerWindow {
    answer_window(deps, listen.silence_sec, listen.max_sec, listen.min_sec)
}

fn planned_beat(utterance: EngineUtterance, progress: EngineProgress) -> RtBeat {
    RtBeat {
        seq: 0,
        utterance,
        part: None,
        audio: None,
        answer: None,
        progress,
    }
}

// ───────────────────────── 开始和续面 ─────────────────────────

/// 开始一场实时对话的面试：开场白加第一道题，都是提前合成好的。
pub(crate) fn open(
    deps: &Deps,
    plan: &InterviewPlan,
    config: &InterviewConfig,
) -> ApiResult<RtState> {
    let (live, first) = live::open(plan, config).map_err(engine_error)?;
    let mut beats: Vec<RtBeat> = first
        .utterances
        .into_iter()
        .map(|utterance| planned_beat(utterance, first.progress))
        .collect();
    if let (Some(listen), Some(last)) = (&first.listen, beats.last_mut()) {
        last.answer = Some(window(deps, listen));
    }
    if beats.is_empty() {
        return Err(ApiError::internal(anyhow::anyhow!(
            "面试的第一步没有要说的话"
        )));
    }
    let mut head = beats.remove(0);
    head.seq = 1;
    Ok(RtState {
        schema_version: RT_SCHEMA_VERSION,
        live,
        seq: 1,
        last: RtStep::Say(head),
        queue: beats,
        thinking: None,
        ending: first.finished,
    })
}

/// 把最近的一步变成协议里的「下一步」：补上语音地址和时长。
pub(crate) async fn current_step(
    deps: &Deps,
    interview_id: Uuid,
    state: &RtState,
) -> ApiResult<NextStep> {
    let beat = match &state.last {
        RtStep::Finish { reason } => {
            return Ok(NextStep::Finish {
                reason: finish_reason(*reason).to_owned(),
            });
        }
        RtStep::Say(beat) => beat,
    };
    // 提前合成的句子，语音记在 interview_utterances 里
    let planned: Vec<String> = std::iter::once(beat)
        .chain(&state.queue)
        .filter(|beat| beat.part.is_none())
        .map(|beat| beat.utterance.key.clone())
        .collect();
    let voiced: Vec<(String, Option<i32>)> = sqlx::query_as(
        "SELECT key, audio_duration_ms FROM interview_utterances
          WHERE interview_id = $1 AND key = ANY($2)
            AND audio_blob_key IS NOT NULL AND synth_status = 'done'",
    )
    .bind(interview_id)
    .bind(&planned)
    .fetch_all(&deps.ctx.db)
    .await?;
    let audio_of = |beat: &RtBeat| -> Option<(String, Option<i64>)> {
        let key = &beat.utterance.key;
        match beat.part {
            Some(part) => beat
                .audio
                .as_ref()
                .map(|audio| (live_audio_url(key, part), Some(audio.duration_ms))),
            None => voiced
                .iter()
                .find(|(voiced_key, _)| voiced_key == key)
                .map(|(_, duration)| (audio_url(key), duration.map(i64::from))),
        }
    };

    let own = audio_of(beat);
    Ok(NextStep::Say {
        seq: beat.seq,
        utterance: Utterance {
            key: beat.utterance.key.clone(),
            kind: utterance_kind(beat.utterance.kind),
            text: beat.utterance.text.clone(),
            audio_url: own.as_ref().map(|(url, _)| url.clone()),
            audio_duration_ms: own.and_then(|(_, duration)| duration),
        },
        expects_answer: beat.answer.is_some(),
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
        preload_audio_urls: state
            .queue
            .iter()
            .filter_map(|queued| audio_of(queued).map(|(url, _)| url))
            .collect(),
    })
}

/// 现场合成的一句的语音地址。同一段话的几句 key 相同，用 `part` 区分。
fn live_audio_url(key: &str, part: u32) -> String {
    format!("{}?part={part}", audio_url(key))
}

/// 续面时该播的那一句。
pub(crate) async fn resume_step(
    deps: &Deps,
    interview_id: Uuid,
    row: &Row,
) -> ApiResult<Option<NextStep>> {
    let Some(state) = parse(interview_id, row)? else {
        return Ok(None);
    };
    let step = current_step(deps, interview_id, &state).await?;
    Ok(Some(at_elapsed(step, row.elapsed_ms(Utc::now()))))
}

// ───────────────────────── 推进 ─────────────────────────

enum Once {
    /// 把状态里最近的这一步发给页面。
    Step(Box<RtState>),
    /// 面试已经结束。
    Finished(NextStep),
    /// 下一句还没准备好。
    Wait,
}

/// 页面上报一句播完了（要听回答的那一句，连同回答一起报），返回下一句。
/// 下一句还没准备好就等着。
pub(crate) async fn advance(
    deps: &Deps,
    interview_id: Uuid,
    request: &AdvanceRequest,
) -> ApiResult<NextStep> {
    let deadline = Instant::now() + ADVANCE_WAIT;
    let mut changes = deps.realtime.watch(interview_id);
    let mut attempts = 0;
    loop {
        // 先记下「看过了」再去读库：读库之后才排进来的句子会让下面的等待马上醒
        changes.borrow_and_update();
        match advance_once(deps, interview_id, request).await? {
            Once::Step(state) => return current_step(deps, interview_id, &state).await,
            Once::Finished(step) => {
                deps.realtime.forget(interview_id);
                return Ok(step);
            }
            Once::Wait => {}
        }
        // 这一轮没想成（比如数据库一时连不上）会马上醒过来再试。一次请求里最多试这么几回，
        // 不然会一直空转；再不行就等到时间返回 503，页面重试时重新来
        if attempts < TURN_ATTEMPTS && spawn_turn(deps, interview_id) {
            attempts += 1;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(ApiError::unavailable("面试官还在想怎么接话，请稍后再试")
                .with_code("answer_pending"));
        }
        let _ = tokio::time::timeout(left.min(POLL_EVERY), changes.changed()).await;
    }
}

async fn advance_once(
    deps: &Deps,
    interview_id: Uuid,
    request: &AdvanceRequest,
) -> ApiResult<Once> {
    let mut tx = deps.ctx.db.begin().await?;
    let row = conduct::load(&mut tx, interview_id, true).await?;
    if row.is_finished() {
        return Ok(Once::Finished(row.finish_step()));
    }
    let mut state = match (row.status, parse(interview_id, &row)?) {
        (InterviewStatus::InProgress, Some(state)) => state,
        _ => return Err(ApiError::conflict("面试还没有开始")),
    };

    // 重复上报上一句（断网重试）：返回现在这一句
    if request.seq.checked_add(1) == Some(state.seq) {
        return Ok(Once::Step(Box::new(state)));
    }
    if request.seq != state.seq {
        return Err(ApiError::conflict(format!(
            "现在是第 {} 句，收到的却是第 {} 句的结果",
            state.seq, request.seq
        ))
        .with_code("seq_conflict"));
    }
    let beat = match &state.last {
        RtStep::Say(beat) => beat.clone(),
        RtStep::Finish { .. } => return Ok(Once::Step(Box::new(state))),
    };
    if beat.utterance.key != request.utterance_key {
        return Err(
            ApiError::conflict("上报的这一句和服务端发出的对不上，请刷新页面")
                .with_code("seq_conflict"),
        );
    }

    let times = BeatTimes::from_request(request, row.elapsed_ms(Utc::now()));
    conduct::record_interviewer_turn(
        &mut tx,
        interview_id,
        beat.seq,
        &beat.utterance.key,
        &beat.utterance.text,
        &times,
    )
    .await?;

    let mut changed = false;
    if beat.answer.is_some() {
        let Some(end_reason) = request.end_reason else {
            return Err(ApiError::invalid_field(
                "end_reason",
                "要听回答的那一句，要说明回答是怎么结束的",
            ));
        };
        // 这段回答第一次报上来：记下来，开始想怎么接。重复上报的不再记
        let fresh = state.thinking.is_none() && state.queue.is_empty() && state.ending.is_none();
        if fresh {
            let spoke = end_reason != AnswerEndReason::NoSpeech;
            if spoke {
                conduct::record_candidate_turn(
                    &mut tx,
                    interview_id,
                    beat.seq,
                    answered_key(&state, &beat),
                    end_reason,
                    &times,
                )
                .await?;
            }
            state.thinking = Some(PendingTurn {
                beat_seq: beat.seq,
                elapsed_ms: u64::try_from(times.end_ms()).unwrap_or(0),
                answer_ms: u64::try_from(times.answer_ms()).unwrap_or(0),
                spoke,
            });
            changed = true;
        }
    }

    if !state.queue.is_empty() {
        let mut next = state.queue.remove(0);
        next.seq = state.seq + 1;
        state.seq = next.seq;
        state.last = RtStep::Say(next);
        save(&mut tx, interview_id, &state).await?;
        tx.commit().await?;
        return Ok(Once::Step(Box::new(state)));
    }
    if state.thinking.is_some() {
        if changed {
            save(&mut tx, interview_id, &state).await?;
        }
        tx.commit().await?;
        return Ok(Once::Wait);
    }
    let Some(reason) = state.ending else {
        return Err(ApiError::internal(anyhow::anyhow!(
            "面试 {interview_id} 的实时对话停在第 {} 句，后面既没有话要说也没有结束",
            state.seq
        )));
    };
    state.last = RtStep::Finish { reason };
    save(&mut tx, interview_id, &state).await?;
    conduct::close(
        &mut tx,
        interview_id,
        InterviewStatus::Completed,
        finish_reason(reason),
    )
    .await?;
    tx.commit().await?;
    conduct::release_live(deps, interview_id);
    tracing::info!(%interview_id, reason = finish_reason(reason), "面试结束");
    Ok(Once::Finished(NextStep::Finish {
        reason: finish_reason(reason).to_owned(),
    }))
}

/// 候选人这段话回答的是哪一句。
///
/// 面试官刚问的是题目或者追问，就算在它下面。刚说的是别的（回应候选人的反问、提醒），
/// 回答的还是引擎正在问的那道题。
fn answered_key<'a>(state: &'a RtState, beat: &'a RtBeat) -> &'a str {
    if matches!(
        beat.utterance.kind,
        EngineKind::Question | EngineKind::FollowUp
    ) {
        return &beat.utterance.key;
    }
    state
        .live
        .session
        .current
        .listen
        .as_ref()
        .map_or(beat.utterance.key.as_str(), |listen| {
            listen.utterance_key.as_str()
        })
}

// ───────────────────────── 想怎么接话 ─────────────────────────

/// 在后台想这场面试等着的那段话怎么接。同一场面试同时只有一个在想。
/// 已经有任务在想时什么都不做，返回 `false`。
fn spawn_turn(deps: &Deps, interview_id: Uuid) -> bool {
    if !deps.realtime.begin(interview_id) {
        return false;
    }
    let deps = deps.clone();
    tokio::spawn(async move {
        // 出什么事（包括 panic）都要把「正在想」的标记去掉，不然这场面试再也没人管
        let _running = Running {
            deps: deps.clone(),
            interview_id,
        };
        if let Err(err) = run_turn(&deps, interview_id).await {
            tracing::warn!(%interview_id, reason = %err, "实时对话这一轮没想成，等页面下次上报时再试");
        }
    });
    true
}

struct Running {
    deps: Deps,
    interview_id: Uuid,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.deps.realtime.end(self.interview_id);
    }
}

async fn run_turn(deps: &Deps, interview_id: Uuid) -> ApiResult<()> {
    let mut conn = deps.ctx.db.acquire().await?;
    let row = conduct::load(&mut conn, interview_id, false).await?;
    drop(conn);
    if row.status != InterviewStatus::InProgress {
        return Ok(());
    }
    let Some(state) = parse(interview_id, &row)? else {
        return Ok(());
    };
    let Some(turn) = state.thinking.clone() else {
        return Ok(());
    };
    let (plan, config) = conduct::script(interview_id, &row)?;
    let started = Instant::now();

    // 候选人说了什么
    let text = if turn.spoke {
        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(u64::from(deps.config.settle_timeout_sec));
        match conduct::transcribe_answer(deps, interview_id, deadline).await? {
            Some(answer) => {
                let mut conn = deps.ctx.db.acquire().await?;
                conduct::save_answer_text(&mut conn, interview_id, &answer).await?;
                answer.text
            }
            None => UNHEARD.to_owned(),
        }
    } else {
        String::new()
    };
    let heard_ms = started.elapsed().as_millis();

    // 面试官怎么接
    let (voice_id,): (Option<Uuid>,) =
        sqlx::query_as("SELECT voice_id FROM interviews WHERE id = $1")
            .bind(interview_id)
            .fetch_one(&deps.ctx.db)
            .await?;
    let progress = match &state.last {
        RtStep::Say(beat) => beat.progress,
        RtStep::Finish { .. } => state.live.session.current.progress,
    };
    let mut speaker = Speaker {
        deps,
        interview_id,
        beat_seq: turn.beat_seq,
        voice_id,
        language: config.language,
        progress,
        started,
        action: None,
        parts: 0,
        generated: Vec::new(),
        held: None,
        released: 0,
        first_ready_ms: None,
        stale: false,
        tail_released: false,
        settled_ms: None,
    };
    let candidate = CandidateTurn {
        text,
        elapsed_ms: turn.elapsed_ms,
        answer_ms: turn.answer_ms,
    };
    let model: std::sync::Arc<dyn ChatModel> = deps
        .llm
        .clone()
        .unwrap_or_else(|| std::sync::Arc::new(NoModel));
    let outcome = match converse(
        model.as_ref(),
        &plan,
        &config,
        &state.live,
        &candidate,
        &mut speaker,
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(err) => {
            // 引擎自己出了错：不带模型再走一遍，它会按顺序进入下一题
            tracing::warn!(%interview_id, reason = %err, "实时对话这一轮引擎出错，按顺序问下一题");
            converse(
                &NoModel,
                &plan,
                &config,
                &state.live,
                &candidate,
                &mut speaker,
            )
            .await
            .map_err(engine_error)?
        }
    };
    if speaker.stale {
        return Ok(());
    }

    let action = outcome.action;
    let timing = outcome.timing;
    let spent = outcome.usage;
    let beats = speaker.released + u32::from(speaker.held.is_some());
    let first_ready_ms = speaker.first_ready_ms;
    let settled_ms = speaker.settled_ms;
    finish_turn(deps, interview_id, &turn, speaker, outcome).await?;
    deps.realtime.notify(interview_id);

    conduct::record_turn_usage(
        deps,
        interview_id,
        &plan,
        &config,
        &spent,
        Duration::from_millis(timing.total_ms),
    )
    .await;
    tracing::info!(
        %interview_id,
        action = ?action,
        beats,
        heard_ms,
        decision_ms = timing.decision_ms,
        first_ready_ms,
        settled_ms,
        total_ms = started.elapsed().as_millis(),
        "实时对话接了一轮"
    );
    Ok(())
}

/// 让引擎想怎么接，同时把它陆续给出来的话准备好。
async fn converse(
    model: &dyn ChatModel,
    plan: &InterviewPlan,
    config: &InterviewConfig,
    state: &LiveState,
    candidate: &CandidateTurn,
    speaker: &mut Speaker<'_>,
) -> Result<LiveOutcome, EngineError> {
    let (events, mut incoming) = mpsc::unbounded_channel();
    let respond = async move {
        let mut sink = move |event: LiveEvent| {
            let _ = events.send(event);
        };
        live::respond(model, plan, config, state, candidate, &mut sink).await
    };
    let speak = async {
        while let Some(event) = incoming.recv().await {
            speaker.on_event(event).await;
        }
    };
    let (outcome, ()) = tokio::join!(respond, speak);
    outcome
}

/// 把引擎给出来的话一句一句准备好，排进队列。
///
/// 最后一句播完是听回答还是结束面试，要等引擎说这一轮的话都给出来了（`Done`）才知道。所以手里
/// 总留着最新的一句（`held`），来了下一句或者来了 `Done` 才把它排进去。
struct Speaker<'a> {
    deps: &'a Deps,
    interview_id: Uuid,
    /// 候选人答的是第几拍
    beat_seq: u32,
    voice_id: Option<Uuid>,
    language: Language,
    /// 这一轮想完之前，排进去的句子用的进度
    progress: EngineProgress,
    started: Instant,
    action: Option<LiveAction>,
    /// 这一轮现场生成了几句
    parts: u32,
    /// 这一轮现场生成的话用了哪些 key
    generated: Vec<String>,
    held: Option<RtBeat>,
    released: u32,
    /// 多久之后第一句可以播了
    first_ready_ms: Option<u128>,
    /// 这一轮已经不作数了（面试结束了，或者状态被别的进程改了）
    stale: bool,
    /// 最后一句已经提前排进队列了（引擎发了 `Done`），整轮想完时不用再排
    tail_released: bool,
    /// 多久之后知道这一轮的话都给出来了
    settled_ms: Option<u128>,
}

impl Speaker<'_> {
    async fn on_event(&mut self, event: LiveEvent) {
        match event {
            LiveEvent::Action { action } => self.action = Some(action),
            LiveEvent::Play { utterance } => {
                self.release_held().await;
                self.hold(planned_beat(utterance, self.progress));
            }
            LiveEvent::Speak { key, text } => {
                self.release_held().await;
                self.parts += 1;
                let part = self.parts;
                let audio = self.synthesize(&key, part, &text).await;
                if !self.generated.contains(&key) {
                    self.generated.push(key.clone());
                }
                let kind = match self.action {
                    Some(LiveAction::Probe) => EngineKind::FollowUp,
                    _ => EngineKind::Other,
                };
                self.hold(RtBeat {
                    seq: 0,
                    utterance: EngineUtterance { key, kind, text },
                    part: Some(part),
                    audio,
                    answer: None,
                    progress: self.progress,
                });
            }
            LiveEvent::Done {
                listen, progress, ..
            } => self.settle(listen.as_ref(), progress).await,
            // 引擎以后再加事件，面试间不认识的先不管，免得它一加这里就编译不过
            #[allow(
                unreachable_patterns,
                reason = "现在的事件都处理了，这一支是给引擎以后加的事件留的"
            )]
            _ => {}
        }
    }

    /// 引擎说这一轮的话都给出来了：手里留着的最后一句不用等整轮想完，现在就能排进去。
    ///
    /// 换题时模型这之后还要写上一道题的概括，实测早 0.3 到 0.4 秒。引擎的新状态、现场生成的话
    /// 登记成句子，还是等整轮想完在 [`finish_turn`] 里记。
    async fn settle(&mut self, listen: Option<&Listen>, progress: EngineProgress) {
        self.settled_ms
            .get_or_insert(self.started.elapsed().as_millis());
        let Some(mut beat) = self.held.take() else {
            // 这一轮什么都没说，等整轮想完再看要不要把上一句重说一遍
            return;
        };
        if self.stale {
            return;
        }
        beat.answer = listen.map(|listen| window(self.deps, listen));
        beat.progress = progress;
        match enqueue(self.deps, self.interview_id, self.beat_seq, beat.clone()).await {
            Ok(true) => {
                self.released += 1;
                self.tail_released = true;
                self.deps.realtime.notify(self.interview_id);
            }
            Ok(false) => self.stale = true,
            Err(err) => {
                // 没排进去就还留在手里，整轮想完时和引擎的新状态一起写
                tracing::warn!(interview_id = %self.interview_id, reason = %err, "实时对话的最后一句没能提前排进队列，等这一轮想完再排");
                self.held = Some(beat);
            }
        }
    }

    fn hold(&mut self, beat: RtBeat) {
        self.first_ready_ms
            .get_or_insert(self.started.elapsed().as_millis());
        self.held = Some(beat);
    }

    async fn release_held(&mut self) {
        let Some(beat) = self.held.take() else {
            return;
        };
        if self.stale {
            return;
        }
        match enqueue(self.deps, self.interview_id, self.beat_seq, beat).await {
            Ok(true) => {
                self.released += 1;
                self.deps.realtime.notify(self.interview_id);
            }
            Ok(false) => self.stale = true,
            Err(err) => {
                tracing::warn!(interview_id = %self.interview_id, reason = %err, "实时对话有一句没排进队列");
            }
        }
    }

    /// 合成一句话并存起来。合成不出来返回 `None`，页面只显示文字。
    async fn synthesize(&self, key: &str, part: u32, text: &str) -> Option<LiveAudio> {
        let interview_id = self.interview_id;
        let voice = self.deps.voice.as_ref()?;
        let spoken = match tokio::time::timeout(
            SYNTH_TIMEOUT,
            voice.speak(self.voice_id, self.language, text),
        )
        .await
        {
            Ok(Ok(spoken)) => spoken,
            Ok(Err(reason)) => {
                tracing::warn!(%interview_id, %reason, "现场合成没成功，这一句只显示文字");
                return None;
            }
            Err(_) => {
                tracing::warn!(%interview_id, "现场合成太慢，这一句只显示文字");
                return None;
            }
        };
        let blob_key = blob_key::utterance_audio(interview_id, &format!("{key}.s{part}"));
        if let Err(err) = self.deps.ctx.blobs.put(&blob_key, spoken.wav.into()).await {
            tracing::warn!(%interview_id, reason = %err, "现场合成的语音没存下来，这一句只显示文字");
            return None;
        }
        Some(LiveAudio {
            blob_key,
            duration_ms: clamp_i64(spoken.duration_ms),
        })
    }
}

/// 这一轮是不是还作数：面试还在进行，等着的还是同一段话。
fn still_thinking(row: &Row, state: Option<&RtState>, beat_seq: u32) -> bool {
    row.status == InterviewStatus::InProgress
        && state
            .and_then(|state| state.thinking.as_ref())
            .is_some_and(|turn| turn.beat_seq == beat_seq)
}

/// 把准备好的一句排进队列。这一轮已经不作数时返回 `false`。
async fn enqueue(deps: &Deps, interview_id: Uuid, beat_seq: u32, beat: RtBeat) -> ApiResult<bool> {
    let mut tx = deps.ctx.db.begin().await?;
    let row = conduct::load(&mut tx, interview_id, true).await?;
    let state = parse(interview_id, &row)?;
    if !still_thinking(&row, state.as_ref(), beat_seq) {
        return Ok(false);
    }
    let Some(mut state) = state else {
        return Ok(false);
    };
    state.queue.push(beat);
    save(&mut tx, interview_id, &state).await?;
    tx.commit().await?;
    Ok(true)
}

/// 一轮想完了：排进最后一句，记下引擎的新状态，以及说完之后是听回答还是结束。
async fn finish_turn(
    deps: &Deps,
    interview_id: Uuid,
    turn: &PendingTurn,
    speaker: Speaker<'_>,
    outcome: LiveOutcome,
) -> ApiResult<()> {
    let mut tx = deps.ctx.db.begin().await?;
    let row = conduct::load(&mut tx, interview_id, true).await?;
    let state = parse(interview_id, &row)?;
    if !still_thinking(&row, state.as_ref(), turn.beat_seq) {
        return Ok(());
    }
    let Some(mut state) = state else {
        return Ok(());
    };

    let mut tail = speaker.held;
    // 最后一句已经提前排进去的（带着听不听回答和进度），页面可能都播完了，这里不用再管
    if let Some(listen) = outcome.listen.as_ref().filter(|_| !speaker.tail_released) {
        if tail.is_none() && state.queue.is_empty() {
            // 这一轮什么都没说却要接着听：把刚才那句再说一遍，不让页面干等
            if let RtStep::Say(asked) = &state.last {
                tail = Some(RtBeat {
                    seq: 0,
                    ..asked.clone()
                });
            }
        }
        if let Some(beat) = &mut tail {
            beat.answer = Some(window(deps, listen));
            beat.progress = outcome.progress;
        } else if let Some(beat) = state.queue.last_mut() {
            beat.answer = Some(window(deps, listen));
            beat.progress = outcome.progress;
        }
    }
    state.queue.extend(tail);
    // 只有最后一句之后才听回答。引擎出错重来的那一轮，前面可能已经排进过一句带着听回答的
    if let Some((_, earlier)) = state.queue.split_last_mut() {
        for beat in earlier {
            beat.answer = None;
        }
    }
    state.live = outcome.state;
    state.thinking = None;
    state.ending = outcome.finished;
    save(&mut tx, interview_id, &state).await?;

    // 现场生成的话登记成这场面试的句子，面试后整理记录和出报告时按 key 能找到它问的是什么
    for (index, utterance) in outcome
        .said
        .iter()
        .filter(|utterance| speaker.generated.contains(&utterance.key))
        .enumerate()
    {
        let kind =
            serde_json::to_value(utterance_kind(utterance.kind)).map_err(ApiError::internal)?;
        let seq = LIVE_UTTERANCE_SEQ_BASE + i64::from(turn.beat_seq) * 10 + clamp_i64(index as u64);
        sqlx::query(
            "INSERT INTO interview_utterances
                    (interview_id, key, seq, kind, text, source, synth_status)
             VALUES ($1, $2, $3, $4, $5, 'live', 'skipped')
             ON CONFLICT (interview_id, key) DO UPDATE
                   SET text = EXCLUDED.text, kind = EXCLUDED.kind, seq = EXCLUDED.seq
                 WHERE interview_utterances.source = 'live'",
        )
        .bind(interview_id)
        .bind(&utterance.key)
        .bind(i32::try_from(seq).unwrap_or(i32::MAX))
        .bind(kind.as_str().unwrap_or("other"))
        .bind(&utterance.text)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

// ───────────────────────── 谁在等、谁在想 ─────────────────────────

struct Entry {
    /// 队列每变一次加一，等下一句的请求靠它醒
    changed: watch::Sender<u64>,
    /// 有没有后台任务正在想这场面试怎么接话
    running: bool,
}

/// 进程里记着的实时对话：哪场面试有请求在等下一句，哪场正在想。
#[derive(Default)]
pub(crate) struct Hub {
    entries: Mutex<HashMap<Uuid, Entry>>,
}

impl Hub {
    fn with<R>(&self, interview_id: Uuid, f: impl FnOnce(&mut Entry) -> R) -> R {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        f(entries.entry(interview_id).or_insert_with(|| Entry {
            changed: watch::Sender::new(0),
            running: false,
        }))
    }

    fn existing(&self, interview_id: Uuid, f: impl FnOnce(&mut Entry)) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(entry) = entries.get_mut(&interview_id) {
            f(entry);
        }
    }

    fn watch(&self, interview_id: Uuid) -> watch::Receiver<u64> {
        self.with(interview_id, |entry| entry.changed.subscribe())
    }

    fn notify(&self, interview_id: Uuid) {
        self.existing(interview_id, |entry| {
            entry.changed.send_modify(|count| *count += 1);
        });
    }

    /// 开始想。已经有别的任务在想时返回 `false`。
    fn begin(&self, interview_id: Uuid) -> bool {
        self.with(interview_id, |entry| {
            !std::mem::replace(&mut entry.running, true)
        })
    }

    fn end(&self, interview_id: Uuid) {
        self.existing(interview_id, |entry| {
            entry.running = false;
            entry.changed.send_modify(|count| *count += 1);
        });
    }

    /// 面试结束了，不用再记着它。
    pub(crate) fn forget(&self, interview_id: Uuid) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&interview_id);
    }
}
